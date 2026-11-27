//! Snapshot mode (M1, design §5): no host. Draws N frames of the full editing UI offscreen and
//! writes each as a PNG, with `frames.json` (adapter, scene samples, per-frame timing and the
//! design's revision), keeping the control channel open so M1/M2 scripts can ask `ui.handles`,
//! `ui.worldToScreen`, `document.revision` while it runs and for `--hold-ms` after. A `--script`
//! injects wire input events at given frames through the same translation as leg A.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};
use solidcraft_viewport_gpu::select::Policy;

use super::events::{self, LoopEvent};
use super::stream::{EXIT_DEVICE_LOST, EXIT_OK, EXIT_SETUP};
use super::ui_host::{UiHost, UiOptions};
use crate::adapters::control::{self, ControlChannel};
use crate::adapters::encode;
use crate::adapters::gpu::{self, Gpu};
use crate::adapters::painter::Painted;
use crate::domain::geometry::FrameSize;
use crate::domain::input::{self, Translator};
use crate::domain::pacing::Schedule;
use crate::domain::samples::Msaa;

pub struct SnapshotArgs {
    pub dir: PathBuf,
    pub frames: u32,
    pub open: Option<PathBuf>,
    pub script: Option<PathBuf>,
    pub hold: Duration,
    pub gap: Duration,
    pub ui: UiOptions,
    pub size: FrameSize,
    pub adapter: Policy,
    pub msaa: Msaa,
}

/// One step of a script: wire input events (PROTOCOL.md, "InputEvent") before frame `frame`.
#[derive(Deserialize)]
struct Step {
    frame: u32,
    events: Vec<Value>,
}

/// Longest wait for `--open`'s design to load before the frames start anyway.
const OPEN_WAIT: Duration = Duration::from_secs(120);

pub fn run(args: SnapshotArgs) -> i32 {
    if let Err(e) = std::fs::create_dir_all(&args.dir) {
        eprintln!("render-server: cannot make {}: {e}", args.dir.display());
        return EXIT_SETUP;
    }
    let steps = match args.script.as_deref().map(read_script).transpose() {
        Ok(steps) => steps.unwrap_or_default(),
        Err(e) => {
            eprintln!("render-server: script: {e}");
            return EXIT_SETUP;
        }
    };
    let gpu = match gpu::open(args.adapter, args.msaa, args.size.dpr) {
        Ok(gpu) => gpu,
        Err(e) => {
            eprintln!("render-server: {e}");
            return EXIT_SETUP;
        }
    };
    let (tx, rx) = events::channel();
    let ctx = egui::Context::default();
    events::wake_on_repaint(&ctx, tx);
    // A token from the environment, as the host gives one; else a fresh one, printed: this mode
    // is a local measurement with no host to hand it to.
    let (token, generated) = match control::token_from_env() {
        Some(t) => (t, false),
        None => match control::new_token() {
            Some(t) => (t, true),
            None => {
                eprintln!("render-server: no random token available");
                return EXIT_SETUP;
            }
        },
    };
    let (control, control_rx) = match ControlChannel::open(ctx.clone(), token.clone()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("render-server: control channel: {e}");
            return EXIT_SETUP;
        }
    };
    // The line M1/M2 scripts wait for (PROTOCOL.md, "M1 mode").
    eprintln!("control 127.0.0.1:{}", control.port);
    if generated {
        eprintln!("control-token {token}");
    }
    let ui = UiHost::new(ctx, &gpu, &args.ui, args.size, solidcraft_ui_egui::Services::default(), control_rx);
    let mut snap = Snapshot { gpu, ui, rx, schedule: Schedule::default(), translator: Translator::native() };
    let code = snap.run(&args, &steps);
    control.close();
    code
}

fn read_script(path: &Path) -> Result<Vec<Step>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

struct Snapshot {
    gpu: Gpu,
    ui: UiHost,
    rx: Receiver<LoopEvent>,
    schedule: Schedule,
    translator: Translator,
}

impl Snapshot {
    fn run(&mut self, args: &SnapshotArgs, steps: &[Step]) -> i32 {
        if let Some(file) = &args.open {
            // As a design given at launch opens (`apps/solidcraft/src/main.rs:1453`).
            self.ui.app.open_chosen_file(&file.to_string_lossy());
            let until = Instant::now() + OPEN_WAIT;
            while self.ui.app.file_load.busy() && Instant::now() < until {
                if self.frame(Vec::new()).is_err() {
                    return EXIT_DEVICE_LOST;
                }
                std::thread::sleep(Schedule::min_interval());
            }
        }
        let mut rows = Vec::new();
        for n in 1..=args.frames {
            let events = self.events_for(steps, n);
            let (pass_ms, paint_ms) = match self.frame(events) {
                Ok(t) => t,
                Err(()) => return EXIT_DEVICE_LOST,
            };
            let t = Instant::now();
            let mut rgba = Vec::new();
            let written = self.ui.read_frame(&self.gpu, &mut rgba).and_then(|()| {
                let readback_ms = t.elapsed().as_secs_f32() * 1000.0;
                let t = Instant::now();
                let size = self.ui.size();
                let png = encode::png(&rgba, size.px_w, size.px_h)?;
                let name = format!("frame-{n:04}.png");
                std::fs::write(args.dir.join(&name), png).map_err(|e| e.to_string())?;
                Ok((name, readback_ms, t.elapsed().as_secs_f32() * 1000.0))
            });
            match written {
                Ok((file, readback, encode)) => rows.push(json!({
                    "n": n,
                    "file": file,
                    "timing": {"pass": pass_ms, "paint": paint_ms, "readback": readback, "encode": encode},
                    "revision": self.ui.app.session.design_revision(),
                })),
                Err(e) => {
                    eprintln!("render-server: frame {n}: {e}");
                    return EXIT_SETUP;
                }
            }
            std::thread::sleep(args.gap);
        }
        if let Err(e) = self.write_report(&args.dir, rows) {
            eprintln!("render-server: frames.json: {e}");
            return EXIT_SETUP;
        }
        eprintln!("frames done");
        self.hold(args.hold)
    }

    /// One pass and its paint (screenshots answered): the pass's and the paint's milliseconds.
    fn frame(&mut self, events: Vec<egui::Event>) -> Result<(f32, f32), ()> {
        let pass = self.ui.pass(events);
        self.schedule.ran(Instant::now(), pass.repaint_delay);
        let (painted, paint_ms) = self.ui.paint(&self.gpu, pass.shapes, pass.pixels_per_point);
        if let Painted::Lost(why) = painted {
            eprintln!("render-server: the graphics device was lost: {why}");
            return Err(());
        }
        self.ui.answer_screenshots(&self.gpu, pass.screenshots);
        Ok((pass.ms, paint_ms))
    }

    /// The script's events before frame `n`, as UI events.
    fn events_for(&mut self, steps: &[Step], n: u32) -> Vec<egui::Event> {
        let points_per_css = 1.0 / self.ui.zoom();
        let mut out = Vec::new();
        for step in steps.iter().filter(|s| s.frame == n) {
            let (events, dropped) = input::parse_events(&step.events);
            if dropped > 0 {
                eprintln!("render-server: script frame {n}: {dropped} events not read");
            }
            for ev in events {
                if let input::InputEvent::Focus { focused } = ev {
                    self.ui.set_focused(focused);
                }
                out.extend(self.translator.event(ev, points_per_css));
            }
        }
        out
    }

    /// Keep answering the control channel (passes on demand) for `hold`.
    fn hold(&mut self, hold: Duration) -> i32 {
        let until = Instant::now() + hold;
        loop {
            let now = Instant::now();
            if now >= until {
                return EXIT_OK;
            }
            let wake = [self.schedule.next_pass(), self.ui.pictures.release_at()].into_iter().flatten().fold(until, Instant::min);
            if let Ok(LoopEvent::Repaint(delay)) = self.rx.recv_timeout(wake.saturating_duration_since(now)) {
                self.schedule.repaint_after(Instant::now(), delay);
            }
            self.ui.pictures.release_idle(Instant::now());
            if self.schedule.due(Instant::now()) && self.frame(Vec::new()).is_err() {
                return EXIT_DEVICE_LOST;
            }
        }
    }

    fn write_report(&mut self, dir: &Path, frames: Vec<Value>) -> Result<(), String> {
        let size = self.ui.size();
        let report = json!({
            "adapter": self.gpu.adapter,
            "backend": self.gpu.backend,
            "software": self.gpu.software,
            "samplesRequested": self.gpu.scene_samples,
            // The scene's samples in effect: the UI's own check may have fallen back to 1x.
            "samples": self.ui.app.viewport.gpu.map_or(self.gpu.scene_samples, |g| g.samples),
            "size": {"w": size.css_w, "h": size.css_h, "dpr": size.dpr, "pxW": size.px_w, "pxH": size.px_h},
            "frames": frames,
            // Where the backend can say (Vulkan, D3D12): what the device holds, by label.
            "gpuMemory": gpu::memory(&self.gpu.rs.device).map(|m| m.to_json(16)),
        });
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("frames.json"), text).map_err(|e| e.to_string())
    }
}
