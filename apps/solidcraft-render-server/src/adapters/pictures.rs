//! The model's headless pictures (the opening's preview, a save's thumbnail, `ui.render`, the
//! agent's view image). The desktop app draws them all with a second copy of the viewport's
//! pipelines made at start (`crates/ui-egui/src/gpu.rs:131`, `install_picture_renderer`), which
//! a stream holds for nothing between pictures (on lavapipe ~140 MiB of compiled shaders; freed
//! memory a long-lived process does not always get back). Here small pictures, the ones the UI
//! draws for itself while opening and saving, are drawn by the CPU rasterizer, and only larger
//! ones (the model's renders) by that picture renderer, made for the first and dropped [`IDLE`]
//! after the last.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui_wgpu::wgpu;
use solidcraft_engine::capture::Picture;
use solidcraft_viewport::{Camera, Scene};
use solidcraft_viewport_gpu::capture::Capturer;

/// How long the picture renderer is kept after a picture: a burst of the model's pictures reuses
/// it, a stream that only edits does not hold it.
pub const IDLE: Duration = Duration::from_secs(3);

/// Pictures up to this many pixels are drawn by the CPU rasterizer: the opening's 320×200
/// preview, a save's thumbnail, a mesh export's 512² one (upstream's last-resort renderer; the
/// reply names it, and it draws no analysis colours or highlights, which these never show).
pub const CPU_MAX_PIXELS: u64 = 512 * 512;

/// What a picture renderer is made on.
struct Device {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    lost: Arc<AtomicBool>,
}

#[derive(Default)]
struct Slot {
    capturer: Option<Arc<Capturer>>,
    /// When the last picture was drawn.
    last: Option<Instant>,
}

pub struct Pictures {
    slot: Arc<Mutex<Slot>>,
}

impl Pictures {
    /// Draw this process's pictures on `rs`'s device (the engine's renderer, process-wide).
    pub fn install(rs: &egui_wgpu::RenderState) -> Pictures {
        let slot = Arc::new(Mutex::new(Slot::default()));
        let device = Device { adapter: rs.adapter.clone(), device: rs.device.clone(), queue: rs.queue.clone(), lost: rs.device_lost.flag() };
        let drawing = slot.clone();
        solidcraft_engine::capture::install(Arc::new(move |scene, cam, w, h| draw(&device, &drawing, scene, cam, w, h)));
        Pictures { slot }
    }

    /// When the picture renderer is due to be dropped, if one is held. Never waits: while a
    /// picture renderer is being made the slot is free, and while one is drawing it is too.
    pub fn release_at(&self) -> Option<Instant> {
        let slot = self.slot.try_lock().ok()?;
        slot.capturer.as_ref().and(slot.last).map(|t| t + IDLE)
    }

    /// Drop the picture renderer once [`IDLE`] has passed since the last picture (one drawing
    /// now keeps its own handle until it is done).
    pub fn release_idle(&self, now: Instant) {
        if let Ok(mut slot) = self.slot.try_lock()
            && slot.last.is_some_and(|t| now >= t + IDLE)
            && slot.capturer.take().is_some()
        {
            log::info!("picture renderer released");
        }
    }
}

fn draw(device: &Device, slot: &Mutex<Slot>, scene: &Scene, cam: &Camera, w: u32, h: u32) -> Result<Picture, String> {
    if u64::from(w) * u64::from(h) <= CPU_MAX_PIXELS {
        return Ok(Picture { rgba: solidcraft_viewport::render(scene, cam, w as usize, h as usize).rgba, renderer: "cpu" });
    }
    let held = slot.lock().map_err(poisoned)?.capturer.clone();
    let capturer = match held {
        Some(c) => c,
        // Made outside the lock (it compiles the viewport's pipelines); two pictures racing keep
        // the first one made.
        None => {
            let made = Capturer::new(&device.adapter, device.device.clone(), device.queue.clone()).map_err(|e| format!("picture renderer: {e}"))?;
            let made = Arc::new(made.watching(device.lost.clone()));
            slot.lock().map_err(poisoned)?.capturer.get_or_insert(made).clone()
        }
    };
    let rgba = capturer.rgba(scene, cam, w, h).map_err(|e| e.to_string());
    slot.lock().map_err(poisoned)?.last = Some(Instant::now());
    Ok(Picture { rgba: rgba?, renderer: capturer.renderer() })
}

fn poisoned<E>(_: E) -> String {
    "the picture renderer is poisoned".to_string()
}
