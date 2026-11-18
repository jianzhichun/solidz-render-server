//! The editing UI with no window: upstream's `SolveApp` in `embed_editor` mode, assembled as the
//! desktop app does it (`apps/solidcraft/src/main.rs:1312-1397`) minus everything window-, file-
//! or update-related, driven one pass at a time by synthesized `RawInput` and painted offscreen.

use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use solidcraft_control::ControlRequest;
use solidcraft_engine::Session;
use solidcraft_ui_egui::{Services, SolveApp};

use crate::adapters::gpu::{Gpu, STENCIL_BITS};
use crate::adapters::painter::{OffscreenPainter, Painted};
use crate::adapters::pictures::Pictures;
use crate::adapters::readback::Readback;
use crate::domain::geometry::FrameSize;

/// Why upstream's Assistant is refused here (shown as "<command>当前不可用：<this>"): the host's chat
/// is the assistant, and an agent started from this UI would write and export around the host
/// (no `ask_save`, so no trial mark or read-only check).
const ASSISTANT_REFUSED: &str = "这里不提供助手，请用对话里的助手";

/// What the host chose for the UI (flags).
#[derive(Clone, Debug)]
pub struct UiOptions {
    /// A registered locale id (`zh`, `crates/ui-text/src/lib.rs:160-175`).
    pub lang: String,
    pub dark: bool,
}

/// One pass's results the loop acts on.
pub struct Pass {
    pub shapes: Vec<egui::epaint::ClippedShape>,
    pub pixels_per_point: f32,
    pub platform: egui::PlatformOutput,
    /// When the UI wants the next pass (`Duration::MAX`: not until something happens).
    pub repaint_delay: Duration,
    /// `ui.screenshot` and the like: the frame painted after this pass, read back.
    pub screenshots: Vec<egui::ScreenshotCallback>,
    /// The pass set an error on the status line (an export that failed).
    pub status_error: bool,
    pub ms: f32,
}

pub struct UiHost {
    pub ctx: egui::Context,
    pub app: SolveApp,
    started: Instant,
    size: FrameSize,
    focused: bool,
    theme: egui::Theme,
    max_texture_side: usize,
    /// Texture changes since the last paint: passes that are not painted (pane hidden) keep
    /// theirs for the next one, as eframe's `pending_deltas` do.
    deltas: egui::TexturesDelta,
    painter: OffscreenPainter,
    shots: Readback,
    pub pictures: Pictures,
}

impl UiHost {
    /// The UI on `gpu`, its pickers from `services`, its control channel's requests on `control`.
    /// Slow: the viewport's shader compile runs here, before the first frame (the desktop runs it
    /// behind a starting page; the pane shows the viewer meanwhile).
    pub fn new(ctx: egui::Context, gpu: &Gpu, opts: &UiOptions, size: FrameSize, services: Services, control: Receiver<ControlRequest>) -> Self {
        // Before any font or app, as `main` does (`apps/solidcraft/src/main.rs:1099-1102`);
        // `SolveApp::new` repeats them, and later calls find them installed (`lib.rs:917-925`).
        solidcraft_ui_egui::i18n::install();
        solidcraft_look::install_tables();
        solidcraft_ui_egui::workspace::install_tabs();
        solidcraft_ui_egui::navigation_slots::install();
        // On this (the UI's) thread: no Assistant panel, key, search row or agent process.
        solidcraft_ui_egui::brand::refuse_commands(&["ui.assistant"], ASSISTANT_REFUSED);
        // Detached panels become in-window windows: there is no OS to give them windows
        // (`crates/egui-0.37.0/src/context.rs:4542`; the default, said out loud).
        ctx.set_embed_viewports(true);
        // `apps/solidcraft/src/main.rs:1316-1317`: the bundled faces, then the system's (CJK).
        solidcraft_ui_egui::fonts::setup_bundled_fonts(&ctx);
        solidcraft_ui_egui::fonts::load_system_fonts_in_background(&ctx);
        // `crates/ui-egui/src/lib.rs:917`.
        let mut app = SolveApp::new(Session::default(), services);
        // #8517 (`lib.rs:603-607`, applied in `cad_ui` `:3799-3812`): no app bar, start page,
        // New/Open/Save/Close; ribbon, browser, timeline and properties stay.
        app.embed_editor = true;
        // Slow edits off the UI thread: the anti-freeze guarantee (`apps/solidcraft/src/main.rs:1325`).
        app.apply_off_thread = Some(solidcraft_ui_egui::dialogs::APPLY_AT_ONCE);
        app.preferences.language = opts.lang.clone();
        app.preferences.theme = if opts.dark { "dark" } else { "light" }.to_string();
        app.services.graphics_backend = gpu.backend;
        app.services.graphics_adapters = gpu
            .rs
            .available_adapters
            .iter()
            .map(|a| {
                let info = a.get_info();
                (info.name.clone(), format!("{} ({:?}, {:?})", info.name, info.device_type, info.backend))
            })
            .collect();
        // `apps/solidcraft/src/main.rs:1554-1600` (`view_step`, `gpu_ready`), synchronously.
        let built = SolveApp::build_wgpu_split(&gpu.rs, gpu.depth_bits, STENCIL_BITS, gpu.scene_samples, !app.cpu_meshing);
        // The model's pictures on this device, their renderer made when one is asked for.
        let pictures = Pictures::install(&gpu.rs);
        app.adopt_wgpu(&gpu.rs, built);
        app.msaa_supported = gpu.supported_samples.clone();
        app.preferences.msaa = gpu.scene_samples;
        solidcraft_ui_egui::prefs::check_msaa(&mut app, gpu.scene_samples);
        // No design given: the empty design takes Units of new designs (`main.rs:1458`).
        solidcraft_ui_egui::prefs::apply_new_design(&mut app);
        // `lib.rs:1412`: requests are executed on this thread by `drain_control` → `run_control` →
        // `self.session.execute`, the same session the UI draws.
        let app = app.with_control(control);
        UiHost {
            max_texture_side: gpu.rs.device.limits().max_texture_dimension_2d as usize,
            ctx,
            app,
            started: Instant::now(),
            size,
            focused: true,
            theme: if opts.dark { egui::Theme::Dark } else { egui::Theme::Light },
            deltas: egui::TexturesDelta::default(),
            painter: OffscreenPainter::default(),
            shots: Readback::default(),
            pictures,
        }
    }

    pub fn size(&self) -> FrameSize {
        self.size
    }

    pub fn set_size(&mut self, size: FrameSize) {
        self.size = size;
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// CSS px per UI point: the UI's own zoom (1 unless a preference changes it).
    pub fn zoom(&self) -> f32 {
        self.ctx.zoom_factor()
    }

    /// One UI pass over `events` (mirrors eframe: `native/epi_integration.rs:337-362,414-425`).
    pub fn pass(&mut self, events: Vec<egui::Event>) -> Pass {
        let t0 = Instant::now();
        let zoom = self.zoom();
        let points = egui::vec2(self.size.css_w as f32 / zoom, self.size.css_h as f32 / zoom);
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, points);
        // `crates/egui-0.37.0/src/data/input/viewport_info.rs:28-87`. Never minimized or occluded,
        // also while the pane is hidden: the model's screenshots must still be answered
        // (`crates/control-egui/src/window.rs:8`).
        let info = egui::ViewportInfo {
            native_pixels_per_point: Some(self.size.dpr),
            monitor_size: Some(points),
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            minimized: Some(false),
            maximized: Some(false),
            fullscreen: Some(false),
            focused: Some(self.focused),
            occluded: Some(false),
            ..Default::default()
        };
        // `crates/egui-0.37.0/src/data/input/raw_input.rs:18-111`.
        let mut raw = egui::RawInput {
            viewport_id: egui::ViewportId::ROOT,
            viewports: std::iter::once((egui::ViewportId::ROOT, info)).collect(),
            screen_rect: Some(rect),
            max_texture_side: Some(self.max_texture_side),
            time: Some(self.started.elapsed().as_secs_f64()),
            events,
            focused: self.focused,
            system_theme: Some(self.theme),
            ..Default::default()
        };
        // As both upstream hosts do before the app's hook (`apps/solidcraft/src/main.rs:319`,
        // `apps/solidcraft-web/src/main.rs:1155`): releases a picker or lost focus kept.
        solidcraft_ui_egui::window_chrome::release_stale_input(&self.ctx, &mut raw);
        // `lib.rs:3632`: the control channel's synthetic events, finite input, click-after-typing.
        self.app.raw_input_hook(&mut raw);
        let status_before = self.app.status.as_ref().map(|s| s.1);
        let app = &mut self.app;
        // `crates/egui-0.37.0/src/context.rs:950`; `lib.rs:3526,3748`.
        let out = self.ctx.run_ui(raw, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        let status_error = self.app.status.as_ref().is_some_and(|(_, at, error)| *error && Some(*at) != status_before);
        let egui::FullOutput { platform_output, textures_delta, shapes, pixels_per_point, mut viewport_output } = out;
        self.deltas.append(textures_delta);
        // `crates/egui-0.37.0/src/viewport.rs:1272-1303`.
        let root = viewport_output.remove(&egui::ViewportId::ROOT);
        let repaint_delay = root.as_ref().map_or(Duration::MAX, |v| v.repaint_delay);
        let mut screenshots = Vec::new();
        for command in root.into_iter().flat_map(|v| v.commands) {
            match command {
                egui::ViewportCommand::Screenshot(callback) => screenshots.push(callback),
                // The pane is not a window: close, minimise, title, focus, cursor warp, size.
                other => log::debug!("viewport command ignored: {other:?}"),
            }
        }
        Pass {
            shapes,
            pixels_per_point,
            platform: platform_output,
            repaint_delay,
            screenshots,
            status_error,
            ms: t0.elapsed().as_secs_f32() * 1000.0,
        }
    }

    /// Tessellate and paint a pass into the offscreen texture; the paint's milliseconds.
    pub fn paint(&mut self, gpu: &Gpu, shapes: Vec<egui::epaint::ClippedShape>, pixels_per_point: f32) -> (Painted, f32) {
        let t0 = Instant::now();
        // As `eframe native/wgpu_integration.rs:822`.
        let prims = self.ctx.tessellate(shapes, pixels_per_point);
        // The theme's panel colour, opaque: the UI covers the pane, and the page draws frames
        // over each other.
        let mut clear = self.ctx.global_style().visuals.panel_fill.to_normalized_gamma_f32();
        clear[3] = 1.0;
        let painted = self.painter.paint(&gpu.rs, self.size.px(), pixels_per_point, clear, &prims, &mut self.deltas);
        (painted, t0.elapsed().as_secs_f32() * 1000.0)
    }

    pub fn texture(&self) -> Option<&egui_wgpu::wgpu::Texture> {
        self.painter.texture()
    }

    /// The texture last painted, as tight opaque RGBA rows (snapshots, screenshots). Its own
    /// buffer: the frame stream's comparison is untouched.
    pub fn read_frame(&mut self, gpu: &Gpu, out: &mut Vec<u8>) -> Result<(), String> {
        let texture = self.painter.texture().ok_or("nothing painted yet")?;
        self.shots.read(&gpu.rs, texture, self.size.px(), out)?;
        for px in out.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        Ok(())
    }

    /// Answer `ui.screenshot` and the like with the frame just painted (egui's callback API,
    /// `crates/egui-0.37.0/src/data/screenshot_callback.rs:25-39`, as eframe hands it the
    /// window's capture). A read that fails drops them: their request times out and says so
    /// (`crates/control-egui/src/screenshot.rs:126-136`).
    pub fn answer_screenshots(&mut self, gpu: &Gpu, callbacks: Vec<egui::ScreenshotCallback>) {
        if callbacks.is_empty() {
            return;
        }
        let mut rgba = Vec::new();
        let read = self.read_frame(gpu, &mut rgba);
        // Screenshots are rare in a stream: their read-back buffer (a whole frame of memory the
        // GPU can write) is not kept between them.
        self.shots = Readback::default();
        match read {
            Ok(()) => {
                let image = Arc::new(egui::ColorImage::from_rgba_unmultiplied([self.size.px_w as usize, self.size.px_h as usize], &rgba));
                for callback in callbacks {
                    callback.complete(image.clone());
                }
            }
            Err(e) => log::warn!("screenshot: {e}"),
        }
    }
}
