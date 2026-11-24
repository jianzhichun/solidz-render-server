//! The painted texture back on the CPU: one copy into a reused `MAP_READ` buffer with rows padded
//! to 256 bytes (as `vendor/egui/crates/egui-wgpu/src/capture.rs:122-190` and
//! `crates/viewport-gpu/src/capture.rs:299-337` do), a blocking wait, and the mapped rows handed
//! to the caller as they are: the frame stream compares and copies straight from them, so no
//! second full frame is kept on the CPU. Synchronous on purpose (lowest latency for one frame).

use std::sync::mpsc;
use std::time::Duration;

use egui_wgpu::wgpu;

use crate::domain::damage::Pixels;

/// Longest wait for the GPU to finish a frame before calling the device stuck.
const WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Readback {
    buffer: Option<(wgpu::Buffer, [u32; 2], u32)>,
}

impl Readback {
    /// Copy `texture` (`size` device px, `Rgba8Unorm`) into the buffer, wait for it, and run `f`
    /// on the mapped rows (RGBA, rows `stride` bytes apart).
    pub fn with_mapped<R>(
        &mut self,
        rs: &egui_wgpu::RenderState,
        texture: &wgpu::Texture,
        size: [u32; 2],
        f: impl FnOnce(Pixels<'_>) -> R,
    ) -> Result<R, String> {
        let [w, h] = size;
        let padded = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        if self.buffer.as_ref().is_none_or(|(_, s, _)| *s != size) {
            // The old one goes before the new one is made.
            self.buffer = None;
            let buffer = rs.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("render-server readback"),
                size: u64::from(padded) * u64::from(h),
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.buffer = Some((buffer, size, padded));
        }
        let Some((buffer, _, _)) = self.buffer.as_ref() else { return Err("no readback buffer".into()) };
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
        let out = {
            // `buffer.rs:620`: a `Result` in wgpu 30.
            let data = slice.get_mapped_range().map_err(|e| format!("reading the frame: {e}"))?;
            let pixels = Pixels { data: &data[..], stride: padded as usize, w, h };
            pixels.is_whole().then(|| f(pixels))
        };
        buffer.unmap();
        out.ok_or_else(|| "the frame read back short".to_string())
    }

    /// The texture as tight RGBA rows in `out`.
    pub fn read(&mut self, rs: &egui_wgpu::RenderState, texture: &wgpu::Texture, size: [u32; 2], out: &mut Vec<u8>) -> Result<(), String> {
        self.with_mapped(rs, texture, size, |pixels| pixels.copy_tight(out))
    }
}
