//! One frame's paint into the offscreen texture: the calls egui-wgpu's own window painter makes,
//! in the same order (`vendor/egui/crates/egui-wgpu/src/winit.rs:680-1095`, `Painter::paint_frame`), with our
//! texture where the window's surface texture was. The 3D viewport is a paint callback
//! (`crates/viewport-egui/src/lib.rs:134,156`), so the model is drawn by this same pass.

use egui_wgpu::wgpu;

/// The texture frames are drawn into and read back from.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: [u32; 2],
}

/// How a paint ended.
pub enum Painted {
    /// Submitted; the texture holds the frame once the submission completes (a read-back waits for it).
    Done,
    /// The graphics device is gone (`DeviceLost`): nothing more can be drawn.
    Lost(String),
}

#[derive(Default)]
pub struct OffscreenPainter {
    target: Option<Target>,
}

impl OffscreenPainter {
    /// The texture last painted, if any.
    pub fn texture(&self) -> Option<&wgpu::Texture> {
        self.target.as_ref().map(|t| &t.texture)
    }

    /// Paint `prims` at `size` device pixels, applying `deltas` (the textures egui set and freed
    /// since the last paint) around it. `clear`: the background in gamma space.
    pub fn paint(
        &mut self,
        rs: &egui_wgpu::RenderState,
        size: [u32; 2],
        pixels_per_point: f32,
        clear: [f32; 4],
        prims: &[egui::ClippedPrimitive],
        deltas: &mut egui::TexturesDelta,
    ) -> Painted {
        // `vendor/egui/crates/egui-wgpu/src/lib.rs:144-221`: the device-lost flag, the probe before uploads
        // (a queue write is the first call to meet a lost device and panics inside wgpu), and
        // `survive` for a loss that lands mid-frame (winit.rs:863-880).
        let lost = &rs.device_lost;
        let compositor = rs.renderer.read().frame_compositor.clone();
        if lost.is_lost() || lost.probe(&rs.device, &rs.queue) {
            if let Some(effect) = &compositor {
                effect.suspend();
            }
            return Painted::Lost(lost.reason().unwrap_or_default());
        }
        self.ensure_target(&rs.device, rs.target_format, size);
        let Some(target) = self.target.as_ref() else { return Painted::Lost("no target".into()) };
        let drawn = lost.survive(&rs.device, &rs.queue, || paint_into(rs, target, pixels_per_point, clear, prims, deltas, compositor.as_deref()));
        match drawn {
            Some(()) => Painted::Done,
            None => Painted::Lost(lost.reason().unwrap_or_default()),
        }
    }

    /// A texture of `size` (re)made when the size changes. `COPY_SRC`: it is read back.
    fn ensure_target(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render-server frame"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.target = Some(Target { texture, view, size });
    }
}

fn paint_into(
    rs: &egui_wgpu::RenderState,
    target: &Target,
    pixels_per_point: f32,
    clear: [f32; 4],
    prims: &[egui::ClippedPrimitive],
    deltas: &mut egui::TexturesDelta,
    compositor: Option<&dyn egui_wgpu::FrameCompositor>,
) {
    // winit.rs:756-758: the compositor (the view's blur, `crates/ui-kit/src/blur_native.rs:683`)
    // selects the viewport before any preparation.
    if let Some(effect) = compositor {
        effect.begin_viewport(egui::ViewportId::ROOT.0.value());
    }
    let mut encoder = rs.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("render-server frame") });
    // `vendor/egui/crates/egui-wgpu/src/renderer.rs:129`.
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: target.size, pixels_per_point };
    // winit.rs:900-920: textures first, then the buffers, which run the paint callbacks' prepare
    // and hand back their command buffers (and tell the compositor the screen, renderer.rs:1030).
    let user_buffers = {
        let mut renderer = rs.renderer.write();
        for (id, image_deltas) in deltas.set.drain() {
            for image_delta in image_deltas {
                renderer.update_texture(&rs.device, &rs.queue, id, &image_delta);
            }
        }
        renderer.update_buffers(&rs.device, &rs.queue, &mut encoder, prims, &screen)
    };
    {
        let renderer = rs.renderer.read();
        // winit.rs:970-984: a one-sample, depthless pass may be composed by the installed effect.
        let composed =
            renderer.frame_compositor.as_ref().is_some_and(|effect| effect.render(&renderer, &mut encoder, &target.view, prims, &screen, clear));
        if !composed {
            // winit.rs:985-1043, without MSAA or depth (the UI pass has neither).
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render-server egui"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: clear[0] as f64, g: clear[1] as f64, b: clear[2] as f64, a: clear[3] as f64 }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // renderer.rs:543; the pass outlives no borrow of the encoder (winit.rs:1036-1042).
            renderer.render(&mut pass.forget_lifetime(), prims, &screen);
        }
    }
    // winit.rs:1056-1066: the callbacks' buffers, then ours.
    rs.queue.submit(user_buffers.into_iter().chain([encoder.finish()]));
    // winit.rs:1073-1075: cached pixels are valid once their producer was submitted.
    if let Some(effect) = compositor {
        effect.submitted();
    }
    // winit.rs:1080-1086: freed after the submit that may still use them.
    let mut renderer = rs.renderer.write();
    for id in deltas.free.drain() {
        renderer.free_texture(&id);
    }
}
