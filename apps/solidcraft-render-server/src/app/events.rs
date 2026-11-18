//! The one channel the frame loop waits on. The loop thread owns the UI and its context
//! (`Services` holds closures that are not `Send`); every other thread (the leg-A reader, the
//! control server's connections through the context's repaint callback) only sends here.

use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use crate::adapters::link::Incoming;

pub enum LoopEvent {
    Host(Incoming),
    /// The UI asked to be drawn again after `delay` (a control request, a finished job, a timer).
    Repaint(Duration),
}

pub fn channel() -> (Sender<LoopEvent>, Receiver<LoopEvent>) {
    mpsc::channel()
}

/// Route the context's repaint requests to the loop (`crates/egui-0.37.0/src/context.rs:2268`):
/// called with the context locked, so it only sends.
pub fn wake_on_repaint(ctx: &egui::Context, tx: Sender<LoopEvent>) {
    ctx.set_request_repaint_callback(move |info| {
        let _ = tx.send(LoopEvent::Repaint(info.delay));
    });
}
