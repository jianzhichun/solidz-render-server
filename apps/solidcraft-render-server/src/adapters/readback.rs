//! The painted texture back on the CPU as tight RGBA rows: one copy into a reused `MAP_READ`
//! buffer with rows padded to 256 bytes (as `vendor/egui-wgpu/src/capture.rs:122-190` and
//! `crates/viewport-gpu/src/capture.rs:299-337` do), then a blocking wait. Synchronous on purpose
//! in v1 (lowest latency for one frame); a pipelined ring is an M3 option.

use std::sync::mpsc;
use std::time::Duration;

use egui_wgpu::wgpu;

/// Longest wait for the GPU to finish a frame before calling the device stuck.
const WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Readback {
    buffer: Option<(wgpu::Buffer, [u32; 2], u32)>,
}

impl Readback {
    /// Copy `texture` (`size` device px, Rgba8Unorm or Bgra8Unorm) into `out` as tight RGBA rows.
    pub fn read(&mut self, rs: &egui_wgpu::RenderState, texture: &wgpu::Texture, size: [u32; 2], out: &mut Vec<u8>) -> Result<(), String> {
        let [w, h] = size;
        let row = w * 4;
        let padded = row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        if self.buffer.as_ref().is_none_or(|(_, s, _)| *s != size) {
            let buffer = rs.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("render-server readback"),
                size: u64::from(padded) * u64::from(h),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.buffer = Some((buffer, size, padded));
        }
        let Some((buffer, _, padded)) = self.buffer.as_ref() else { return Err("no readback buffer".into()) };
        let padded = *padded;
        let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render-server readback") });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(padded), rows_per_image: Some(h) },
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        let submission = rs.queue.submit([encoder.finish()]);
        // Waiting on a device the submission found lost would panic inside wgpu.
        if rs.device_lost.is_lost() {
            return Err("the graphics device was lost".into());
        }
        let slice = buffer.slice(..);
        let (tx, rx) = mpsc::channel();
        // `wgpu-30.0.1/src/api/buffer.rs:579`.
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        // `wgpu-30.0.1/src/api/device.rs:95`; the form `crates/gpu/src/path/mod.rs:309` uses.
        rs.device
            .poll(wgpu::PollType::Wait { submission_index: Some(submission), timeout: Some(WAIT) })
            .map_err(|e| format!("waiting for the frame: {e}"))?;
        match rx.recv_timeout(WAIT) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("mapping the frame: {e}")),
            Err(_) => return Err("the frame was never mapped".into()),
        }
        let copied = {
            // `buffer.rs:620`: a `Result` in wgpu 30.
            let data = slice.get_mapped_range().map_err(|e| format!("reading the frame: {e}"))?;
            out.clear();
            out.reserve(row as usize * h as usize);
            for r in data.chunks(padded as usize).take(h as usize) {
                out.extend_from_slice(r.get(..row as usize).unwrap_or_default());
            }
            out.len() == row as usize * h as usize
        };
        buffer.unmap();
        if !copied {
            return Err("the frame read back short".into());
        }
        if texture.format() == wgpu::TextureFormat::Bgra8Unorm {
            for px in out.as_chunks_mut::<4>().0 {
                px.swap(0, 2);
            }
        }
        Ok(())
    }
}
