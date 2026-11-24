//! Leg A: the one loopback TCP connection to the host that started this process (PROTOCOL.md,
//! "Legs and framing"). Not stdio: upstream code prints to stdout outside tests, which would
//! corrupt a binary stream (design §2.1). A reader thread turns envelopes into messages; a
//! writer thread sends ours, so the frame loop never blocks on the socket.

use std::io::{BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::domain::envelope::{self, ReadError};
use crate::domain::wire::{self, HostMsg, Refusal, ServerMsg};

/// What the reader saw.
pub enum Incoming {
    Msg(HostMsg),
    /// The host broke the protocol: send this `error`, then close.
    Refused(Refusal),
    /// The connection closed or failed.
    Closed(String),
}

/// Sends our messages, in order. Cheap to clone (the UI's pickers hold one).
#[derive(Clone)]
pub struct Outbox {
    tx: Sender<Option<Vec<u8>>>,
}

impl Outbox {
    pub fn send(&self, msg: &ServerMsg) {
        self.send_with(msg, &[]);
    }

    /// A message with a payload (`frame`).
    pub fn send_with(&self, msg: &ServerMsg, payload: &[u8]) {
        let _ = self.tx.send(Some(envelope::leg_a(&wire::header(msg), payload)));
    }

    /// Nothing more after what is queued: the writer sends it and closes our half.
    pub fn close(&self) {
        let _ = self.tx.send(None);
    }
}

/// The writer thread, to wait for after [`Outbox::close`] (the last message is `bye`).
pub struct Writer {
    handle: JoinHandle<()>,
}

impl Writer {
    pub fn join(self) {
        let _ = self.handle.join();
    }
}

/// Connect to the host's listener; `on_incoming` hears every message it sends, on the reader
/// thread (it only forwards them to the frame loop).
pub fn connect(addr: SocketAddr, on_incoming: impl Fn(Incoming) + Send + 'static) -> std::io::Result<(Outbox, Writer)> {
    let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(5))?;
    // Every input event and every frame is latency: no Nagle delay.
    stream.set_nodelay(true)?;
    let reader = stream.try_clone()?;
    std::thread::Builder::new().name("leg-a reader".into()).spawn(move || read_loop(reader, on_incoming))?;
    let (tx, rx) = mpsc::channel::<Option<Vec<u8>>>();
    let handle = std::thread::Builder::new().name("leg-a writer".into()).spawn(move || write_loop(stream, rx))?;
    Ok((Outbox { tx }, Writer { handle }))
}

fn read_loop(stream: TcpStream, on_incoming: impl Fn(Incoming)) {
    let mut r = BufReader::with_capacity(64 << 10, stream);
    loop {
        let incoming = match envelope::read_leg_a(&mut r) {
            Ok(Some(env)) => {
                if !env.payload.is_empty() {
                    log::debug!("leg A: {} payload bytes on a host message ignored", env.payload.len());
                }
                match wire::parse_host(&env.header) {
                    Ok(msg) => Incoming::Msg(msg),
                    Err(refusal) => Incoming::Refused(refusal),
                }
            }
            Ok(None) => Incoming::Closed("the host closed the connection".into()),
            Err(ReadError::Io(e)) => Incoming::Closed(format!("the connection failed: {e}")),
            Err(ReadError::Refused(refusal)) => Incoming::Refused(refusal),
        };
        let last = !matches!(incoming, Incoming::Msg(_));
        on_incoming(incoming);
        if last {
            return;
        }
    }
}

fn write_loop(mut stream: TcpStream, rx: Receiver<Option<Vec<u8>>>) {
    while let Ok(Some(bytes)) = rx.recv() {
        if let Err(e) = stream.write_all(&bytes) {
            log::warn!("leg A: write failed: {e}");
            return;
        }
    }
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
}
