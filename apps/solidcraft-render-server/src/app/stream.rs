//! Stream mode: the frame loop with a host on leg A (PROTOCOL.md, "Starting (leg A)" and
//! "Sequence"). Render on demand (design §1.6): the loop sleeps on its channel until input, a
//! host message, the UI's own repaint request or the idle refinement is due; passes keep running
//! while the pane is hidden (control requests, timers, off-thread results), frames do not.

use std::net::SocketAddr;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use solidcraft_viewport_gpu::select::Policy;

use super::events::{self, LoopEvent};
use super::frames::FrameStream;
use super::output::OutputRelay;
use super::policy::ReadOnly;
use super::services::Asks;
use super::ui_host::{UiHost, UiOptions};
use crate::adapters::control::{self, ControlChannel};
use crate::adapters::gpu::{self, Gpu};
use crate::adapters::link::{self, Incoming, Outbox, Writer};
use crate::adapters::painter::Painted;
use crate::domain::geometry::FrameSize;
use crate::domain::input::{self, InputEvent, Translator};
use crate::domain::pacing::Schedule;
use crate::domain::samples::Msaa;
use crate::domain::wire::{ByeWhy, HostMsg, PROTOCOL_VERSION, ServerMsg};

/// Exit codes the host may read (it acts on `bye`; these are for logs).
pub const EXIT_OK: i32 = 0;
pub const EXIT_SETUP: i32 = 1;
pub const EXIT_PROTOCOL: i32 = 2;
pub const EXIT_DEVICE_LOST: i32 = 3;
pub const EXIT_PANIC: i32 = 4;

pub struct StreamArgs {
    pub addr: SocketAddr,
    pub ui: UiOptions,
    pub size: FrameSize,
    pub adapter: Policy,
    pub msaa: Msaa,
}

/// How the loop ends: the exit code and the last message to send.
struct End {
    code: i32,
    last: Option<ServerMsg>,
}

impl End {
    fn bye(code: i32, why: ByeWhy, detail: Option<String>) -> Self {
        End { code, last: Some(ServerMsg::Bye { why, detail }) }
    }
}

/// The host's token for this process (`SOLIDCRAFT_RENDER_TOKEN`, 64 lowercase hex).
fn stream_token() -> Option<String> {
    let t = std::env::var("SOLIDCRAFT_RENDER_TOKEN").ok()?;
    (t.len() == 64 && t.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))).then_some(t)
}

pub fn run(args: StreamArgs) -> i32 {
    let Some(token) = stream_token() else {
        eprintln!("render-server: SOLIDCRAFT_RENDER_TOKEN must be 64 lowercase hex characters");
        return EXIT_SETUP;
    };
    let Some(control_token) = control::token_from_env() else {
        eprintln!("render-server: SOLIDCRAFT_CONTROL_TOKEN is missing or too short");
        return EXIT_SETUP;
    };
    let (tx, rx) = events::channel();
    let host_tx = tx.clone();
    let (outbox, writer) = match link::connect(args.addr, move |i| {
        let _ = host_tx.send(LoopEvent::Host(i));
    }) {
        Ok(link) => link,
        Err(e) => {
            eprintln!("render-server: cannot reach the host at {}: {e}", args.addr);
            return EXIT_SETUP;
        }
    };
    let gpu = match gpu::open(args.adapter, args.msaa, args.size.dpr) {
        Ok(gpu) => gpu,
        Err(e) => {
            // No message at all: the host takes only `hello` first (anything else is refused and
            // dropped), and an exit before `ready` is what puts its session on the viewer at once
            // (a `bye` would count as a crash and be retried). The reason goes to stderr, which
            // the host keeps in its log.
            log::error!("{e}");
            return finish(&outbox, writer, None, End { code: EXIT_SETUP, last: None });
        }
    };
    outbox.send(&ServerMsg::Hello {
        v: PROTOCOL_VERSION,
        token,
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        backend: gpu.backend,
        adapter: gpu.adapter.clone(),
        software: gpu.software,
    });
    let ctx = egui::Context::default();
    events::wake_on_repaint(&ctx, tx);
    let (control, control_rx) = match ControlChannel::open(ctx.clone(), control_token) {
        Ok(c) => c,
        // Before `ready`, as a device that does not open: the reason on stderr, no `bye`.
        Err(e) => {
            log::error!("control channel: {e}");
            return finish(&outbox, writer, None, End { code: EXIT_SETUP, last: None });
        }
    };
    let asks = Asks::default();
    let picker_out = outbox.clone();
    let services = asks.services(Rc::new(move |m: ServerMsg| picker_out.send(&m)));
    let mut ui = UiHost::new(ctx, &gpu, &args.ui, args.size, services, control_rx);
    // The host's save places stay off the status line (it says where a file went itself).
    ui.app.shown_status = Some(asks.shown_status());
    let mut stream = Stream {
        gpu,
        ui,
        outbox: outbox.clone(),
        asks,
        frames: FrameStream::default(),
        relay: OutputRelay::default(),
        read_only: ReadOnly::default(),
        schedule: Schedule::default(),
        translator: Translator::native(),
        pending: Vec::new(),
        input_seq: 0,
        painted: None,
        visible: true,
        control_port: control.port,
        stopping: false,
    };
    let end = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stream.run(&rx))) {
        Ok(end) => {
            drop(stream);
            end
        }
        // A panic in the loop (the default hook has printed it on stderr): the host hears `bye
        // error` with it, a crash. Not dropped: a UI that panicked mid-pass may panic again in
        // its destructors, before the host was told.
        Err(panic) => {
            std::mem::forget(stream);
            End::bye(EXIT_PANIC, ByeWhy::Error, Some(panic_text(panic.as_ref())))
        }
    };
    finish(&outbox, writer, Some(control), end)
}

fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    let text = panic.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| panic.downcast_ref::<String>().cloned());
    format!("panic: {}", text.unwrap_or_default())
}

/// Stop the control channel, send the last message, let the writer drain, and say the code.
fn finish(outbox: &Outbox, writer: Writer, control: Option<ControlChannel>, end: End) -> i32 {
    if let Some(control) = control {
        control.close();
    }
    if let Some(last) = &end.last {
        outbox.send(last);
    }
    outbox.close();
    writer.join();
    end.code
}

/// The pass the texture was last painted by: its size (the texture's until the next paint,
/// even after a resize), input sequence and timings, for a frame held for lack of a credit.
#[derive(Clone, Copy)]
struct PaintedPass {
    size: FrameSize,
    input_seq: u64,
    pass_ms: f32,
    paint_ms: f32,
}

struct Stream {
    gpu: Gpu,
    ui: UiHost,
    outbox: Outbox,
    asks: Asks,
    frames: FrameStream,
    relay: OutputRelay,
    read_only: ReadOnly,
    schedule: Schedule,
    translator: Translator,
    /// Input for the next pass, in order.
    pending: Vec<egui::Event>,
    /// The last `input.seq` read (echoed as `frame.inputSeq` by the pass that reads it).
    input_seq: u64,
    painted: Option<PaintedPass>,
    visible: bool,
    control_port: u16,
    stopping: bool,
}

impl Stream {
    fn run(&mut self, rx: &Receiver<LoopEvent>) -> End {
        // What the host sent while the device and the pipelines were being made
        // (`policy`, `resize`, `visible`), before the first frame.
        while let Ok(event) = rx.try_recv() {
            if let Some(end) = self.handle(event) {
                return end;
            }
        }
        if let Some(end) = self.pass(true) {
            return end;
        }
        loop {
            // Checked before waiting too: a `stop` that came in before the first pass must not
            // wait for some later event to be seen.
            if self.stopping {
                return End::bye(EXIT_OK, ByeWhy::Stop, None);
            }
            if let Some(end) = self.wait(rx) {
                return end;
            }
            self.ui.pictures.release_idle(Instant::now());
            if self.stopping {
                return End::bye(EXIT_OK, ByeWhy::Stop, None);
            }
            if self.visible && self.frames.held_and_sendable() {
                self.send_held();
            }
            let now = Instant::now();
            if self.schedule.due(now)
                && let Some(end) = self.pass(false)
            {
                return end;
            }
            if self.visible && self.frames.refine_at().is_some_and(|at| at <= now) {
                match self.frames.refine(&self.ui.size(), self.input_seq) {
                    Ok(Some(out)) => self.outbox.send_with(&out.msg, &out.payload),
                    Ok(None) => {}
                    Err(e) => log::warn!("refinement: {e}"),
                }
            }
        }
    }

    /// Sleep until the next event or deadline (a pass, the refinement, dropping the picture
    /// renderer), then take every event that is there.
    fn wait(&mut self, rx: &Receiver<LoopEvent>) -> Option<End> {
        let refine = if self.visible { self.frames.refine_at() } else { None };
        let deadline = [self.schedule.next_pass(), refine, self.ui.pictures.release_at()].into_iter().flatten().min();
        let first = match deadline {
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(event) => Some(event),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Some(End { code: EXIT_OK, last: None }),
            },
            None => match rx.recv() {
                Ok(event) => Some(event),
                Err(_) => return Some(End { code: EXIT_OK, last: None }),
            },
        };
        for event in first.into_iter().chain(std::iter::from_fn(|| rx.try_recv().ok())) {
            if let Some(end) = self.handle(event) {
                return Some(end);
            }
        }
        None
    }

    fn handle(&mut self, event: LoopEvent) -> Option<End> {
        match event {
            LoopEvent::Repaint(delay) => self.schedule.repaint_after(Instant::now(), delay),
            LoopEvent::Host(Incoming::Msg(msg)) => return self.on_host(msg),
            LoopEvent::Host(Incoming::Refused(refusal)) => {
                log::error!("host broke the protocol: {}", refusal.message);
                return Some(End { code: EXIT_PROTOCOL, last: Some(refusal.into_msg()) });
            }
            LoopEvent::Host(Incoming::Closed(why)) => return Some(End::bye(EXIT_OK, ByeWhy::HostGone, Some(why))),
        }
        None
    }

    fn on_host(&mut self, msg: HostMsg) -> Option<End> {
        match msg {
            HostMsg::Input { seq, events } => {
                let (events, dropped) = input::parse_events(&events);
                if dropped > 0 {
                    log::debug!("input {seq}: {dropped} events this version does not read");
                }
                let points_per_css = 1.0 / self.ui.zoom();
                for ev in events {
                    if let InputEvent::Focus { focused } = ev {
                        self.ui.set_focused(focused);
                    }
                    if let Some(e) = self.translator.event(ev, points_per_css) {
                        self.pending.push(e);
                    }
                }
                self.input_seq = seq;
                self.frames.activity(Instant::now());
                self.schedule.want();
            }
            HostMsg::Resize { w, h, dpr } => {
                self.ui.set_size(FrameSize::new(w, h, dpr));
                self.frames.invalidate();
                self.schedule.want();
            }
            HostMsg::Visible { visible } => {
                self.visible = visible;
                if visible {
                    self.frames.invalidate();
                    self.schedule.want();
                }
            }
            HostMsg::Ack { seq, decode_ms } => {
                log::debug!("frame {seq} on screen, decoded in {decode_ms} ms");
                self.frames.ack();
            }
            HostMsg::Policy { read_only } => {
                self.read_only.set(&mut self.ui.app, read_only);
                self.schedule.want();
            }
            HostMsg::Answer { ask, path } => {
                self.asks.answer(ask, path);
                self.schedule.want();
            }
            HostMsg::Stop { why } => {
                log::info!("stop: {why}");
                if let Some(m) = gpu::memory(&self.gpu.rs.device) {
                    log::info!("gpu memory: {}", m.line(12));
                }
                self.stopping = true;
            }
            HostMsg::Error { code, message } => {
                log::error!("host error {code}: {message}");
                return Some(End { code: EXIT_PROTOCOL, last: None });
            }
        }
        None
    }

    /// One UI pass, its outputs, and (pane visible, or a screenshot asked for, or the first)
    /// its paint and frame. `first`: `ready` goes out once the first frame is drawn, before it.
    fn pass(&mut self, first: bool) -> Option<End> {
        let input_seq = self.input_seq;
        let pass = self.ui.pass(std::mem::take(&mut self.pending));
        self.schedule.ran(Instant::now(), pass.repaint_delay);
        for msg in self.relay.relay(pass.platform, self.ui.zoom()) {
            self.outbox.send(&msg);
        }
        for (ask, answered) in self.asks.take_delivered() {
            self.outbox.send(&ServerMsg::AskDone { ask, ok: answered && !pass.status_error });
        }
        self.read_only.after_pass(&mut self.ui.app);
        if !(self.visible || first || !pass.screenshots.is_empty()) {
            return None;
        }
        let (painted, paint_ms) = self.ui.paint(&self.gpu, pass.shapes, pass.pixels_per_point);
        if let Painted::Lost(why) = painted {
            return Some(End::bye(EXIT_DEVICE_LOST, ByeWhy::DeviceLost, Some(why)));
        }
        self.ui.answer_screenshots(&self.gpu, pass.screenshots);
        if first {
            let size = self.ui.size();
            let samples = self.ui.app.viewport.gpu.map_or(self.gpu.scene_samples, |g| g.samples);
            self.outbox.send(&ServerMsg::Ready { control_port: self.control_port, w: size.px_w, h: size.px_h, dpr: size.dpr, samples });
        }
        self.painted = Some(PaintedPass { size: self.ui.size(), input_seq, pass_ms: pass.ms, paint_ms });
        if self.visible {
            self.send_held();
        }
        None
    }

    /// The frame for the texture last painted, if it changed and a credit is left.
    fn send_held(&mut self) {
        let (Some(p), Some(texture)) = (self.painted, self.ui.texture()) else { return };
        match self.frames.send_painted(&self.gpu.rs, texture, &p.size, p.input_seq, p.pass_ms, p.paint_ms) {
            Ok(Some(out)) => self.outbox.send_with(&out.msg, &out.payload),
            Ok(None) => {}
            Err(e) => log::warn!("frame: {e}"),
        }
    }
}
