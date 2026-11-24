//! The model's headless pictures (the opening's preview, a save's thumbnail, `ui.render`, the
//! agent's view image). The desktop app draws them all with a second copy of the viewport's
//! pipelines made at start (`crates/ui-egui/src/gpu.rs:131`, `install_picture_renderer`), which
//! a stream holds for nothing between pictures (on lavapipe ~140 MiB of compiled shaders; freed
//! memory a long-lived process does not always get back). Here small pictures, the ones the UI
//! draws for itself while opening and saving, are drawn by the CPU rasterizer, and only larger
//! ones (the model's renders) and views a surface analysis shades (the rasterizer draws no
//! analysis colours) by that picture renderer, made for the first and dropped [`IDLE`] after the
//! last. As upstream's, a picture the GPU cannot draw (no picture renderer on this device, the
//! device lost) is drawn by the CPU rasterizer, so a save still writes its thumbnail.

use std::cell::Cell;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui_wgpu::wgpu;
use solidcraft_engine::capture::Picture;
use solidcraft_viewport::{Camera, Scene};
use solidcraft_viewport_gpu::GpuError;
use solidcraft_viewport_gpu::capture::Capturer;

/// How long the picture renderer is kept after a picture: a burst of the model's pictures reuses
/// it, a stream that only edits does not hold it.
pub const IDLE: Duration = Duration::from_secs(3);

/// Pictures up to this many pixels are drawn by the CPU rasterizer: the opening's 320×200
/// preview, a save's thumbnail, a mesh export's 512² one (upstream's last-resort renderer; the
/// reply names it).
pub const CPU_MAX_PIXELS: u64 = 512 * 512;

/// Is a `w`×`h` picture drawn by the CPU rasterizer? Small ones are, unless a surface analysis
/// shades the scene.
fn on_cpu(w: u32, h: u32, analysis: bool) -> bool {
    !analysis && u64::from(w) * u64::from(h) <= CPU_MAX_PIXELS
}

thread_local! {
    /// The picture this thread drew last was a small one the CPU draws by choice ([`on_cpu`]).
    static CPU_BY_CHOICE: Cell<bool> = const { Cell::new(false) };
}

/// Was the picture this thread just drew drawn by the CPU by choice, not for want of a GPU? The
/// engine logs its CPU warning right after, on the same thread (`solidcraft_engine::capture::rgba_with`),
/// and that warning names a GPU that is missing ([`crate::adapters::logging`] leaves it out).
pub fn cpu_by_choice() -> bool {
    CPU_BY_CHOICE.with(Cell::get)
}

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
    /// When a picture was last handed the picture renderer or drawn with it.
    last: Option<Instant>,
    /// This device cannot make a picture renderer: every picture is the CPU rasterizer's.
    unavailable: bool,
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

    /// When the picture renderer is due to be dropped, if one is held. The slot is only ever
    /// locked for a moment (the renderer is made and draws outside it).
    pub fn release_at(&self) -> Option<Instant> {
        let slot = self.slot.lock().ok()?;
        slot.capturer.as_ref().and(slot.last).map(|t| t + IDLE)
    }

    /// Drop the picture renderer once [`IDLE`] has passed since the last picture (one drawing
    /// now keeps its own handle until it is done).
    pub fn release_idle(&self, now: Instant) {
        if let Ok(mut slot) = self.slot.lock()
            && slot.last.is_some_and(|t| now >= t + IDLE)
            && slot.capturer.take().is_some()
        {
            log::info!("picture renderer released");
        }
    }
}

fn draw(device: &Device, slot: &Mutex<Slot>, scene: &Scene, cam: &Camera, w: u32, h: u32) -> Result<Picture, String> {
    let by_choice = on_cpu(w, h, scene.analysis.is_some());
    CPU_BY_CHOICE.with(|c| c.set(by_choice));
    if by_choice {
        log::debug!("a {w}×{h} picture drawn by the CPU rasterizer, as small pictures are here (the GPU is not missing)");
        return Ok(cpu(scene, cam, w, h));
    }
    let Some(capturer) = capturer(device, slot)? else { return Ok(cpu(scene, cam, w, h)) };
    let drawn = capturer.rgba(scene, cam, w, h);
    slot.lock().map_err(poisoned)?.last = Some(Instant::now());
    match drawn {
        // `crates/ui-egui/src/gpu.rs` `picture_result` (#15986): the picture renderer draws an
        // opaque backdrop first, so a picture with no visible pixel was not drawn; the CPU draws it.
        Ok(rgba) if invisible(&rgba) => {
            log::warn!("GPU picture was fully transparent; drawing it with the CPU rasterizer");
            Ok(cpu(scene, cam, w, h))
        }
        Ok(rgba) => Ok(Picture { rgba, renderer: capturer.renderer() }),
        // `crates/ui-egui/src/gpu.rs` `picture`: once the device is lost the CPU draws them.
        Err(GpuError::DeviceLost) => Ok(cpu(scene, cam, w, h)),
        Err(e) => Err(e.to_string()),
    }
}

/// Every pixel of an RGBA picture is fully transparent.
fn invisible(rgba: &[u8]) -> bool {
    !rgba.is_empty() && rgba.len().is_multiple_of(4) && rgba.iter().skip(3).step_by(4).all(|&alpha| alpha == 0)
}

/// The picture renderer held, or one made now; `None` when this device cannot make one.
fn capturer(device: &Device, slot: &Mutex<Slot>) -> Result<Option<Arc<Capturer>>, String> {
    {
        let mut s = slot.lock().map_err(poisoned)?;
        if s.unavailable {
            return Ok(None);
        }
        if let Some(c) = s.capturer.clone() {
            // Taken now: an idle release due this instant does not drop it under this picture.
            s.last = Some(Instant::now());
            return Ok(Some(c));
        }
    }
    // Made outside the lock (it compiles the viewport's pipelines); two pictures racing keep the
    // first one made.
    match Capturer::new(&device.adapter, device.device.clone(), device.queue.clone()) {
        Ok(made) => {
            let made = Arc::new(made.watching(device.lost.clone()));
            let mut s = slot.lock().map_err(poisoned)?;
            s.last = Some(Instant::now());
            Ok(Some(s.capturer.get_or_insert(made).clone()))
        }
        Err(e) => {
            // As `install_picture_renderer` at start: tried once, then the CPU draws them.
            log::warn!("pictures of the model are drawn by the CPU rasterizer: the GPU cannot draw them: {e}");
            slot.lock().map_err(poisoned)?.unavailable = true;
            Ok(None)
        }
    }
}

fn cpu(scene: &Scene, cam: &Camera, w: u32, h: u32) -> Picture {
    Picture { rgba: solidcraft_viewport::render(scene, cam, w as usize, h as usize).rgba, renderer: "cpu" }
}

fn poisoned<E>(_: E) -> String {
    "the picture renderer is poisoned".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Small pictures on the CPU; large ones, and any a surface analysis shades, on the GPU.
    #[test]
    fn which_pictures_the_cpu_draws() {
        assert!(on_cpu(320, 200, false));
        assert!(on_cpu(512, 512, false));
        assert!(!on_cpu(513, 512, false));
        assert!(!on_cpu(320, 200, true));
    }

    /// A GPU picture with no visible pixel goes to the CPU; one visible pixel keeps it.
    #[test]
    fn an_invisible_gpu_picture_is_redrawn() {
        assert!(invisible(&[9, 9, 9, 0, 1, 2, 3, 0]));
        assert!(!invisible(&[9, 9, 9, 0, 1, 2, 3, 255]));
        assert!(!invisible(&[]));
    }
}
