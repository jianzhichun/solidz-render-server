//! The control channel on this process's session: the same server the desktop app runs
//! (`apps/solidcraft/src/control_server.rs:68-70`), with the same protocol
//! (`solidcraft_ui_egui::control_protocol::AppProtocol`), on a free loopback port behind the
//! host's token. No token file and no `control/current.port` discovery file: a user's own
//! `solidcraft-cli mcp` must never find this process, nor take it for the desktop app.

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use solidcraft_control::ControlRequest;
use solidcraft_control::server::{self, Hub};
use solidcraft_engine::MAX_LINE;
use solidcraft_ui_egui::control_protocol::AppProtocol;

pub struct ControlChannel {
    pub port: u16,
    stop: Option<server::Stop>,
}

impl ControlChannel {
    /// Listen on `127.0.0.1:0` with `token`. Each request wakes the UI (`wake` → the context's
    /// repaint, `crates/control/src/server.rs:69`); the UI answers it on its thread
    /// (`SolveApp::drain_control`, `crates/ui-egui/src/lib.rs:3909`), from the requests returned here.
    pub fn open(ctx: egui::Context, token: String) -> std::io::Result<(Self, Receiver<ControlRequest>)> {
        let (tx, rx) = channel::<ControlRequest>();
        let hub = Arc::new(Hub::new(AppProtocol, move || ctx.request_repaint(), MAX_LINE));
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        // `crates/control/src/server.rs:103`; not patient: requests wait as long as the desktop's.
        let stop = hub.listen_stoppable(listener, token, tx, false)?;
        Ok((ControlChannel { port, stop: Some(stop) }, rx))
    }

    /// Close the port and every connection on it (at most a couple of seconds).
    pub fn close(mut self) {
        if let Some(stop) = self.stop.take() {
            stop.stop();
        }
    }
}

/// The control token the host gave (`SOLIDCRAFT_CONTROL_TOKEN`, docs/control-protocol.md:519-523),
/// if long enough (`crates/control/src/auth.rs:126`).
pub fn token_from_env() -> Option<String> {
    solidcraft_engine::control_auth::APP.env_token()
}

/// A fresh random token (32 bytes as hex, `crates/control/src/server.rs:212`).
pub fn new_token() -> Option<String> {
    server::new_token()
}
