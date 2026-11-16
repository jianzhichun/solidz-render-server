//! `log` records to stderr, one line each (the host keeps stderr in its engine log). No logger
//! crate is in the tree, and this one needs no more. `SOLIDCRAFT_RENDER_LOG=error|warn|info|debug`
//! (default `warn`; wgpu's own `info` lines are many).

use log::{Level, LevelFilter, Log, Metadata, Record};

struct Stderr;

static LOGGER: Stderr = Stderr;

impl Log for Stderr {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let level = match record.level() {
                Level::Error => "error",
                Level::Warn => "warn",
                Level::Info => "info",
                Level::Debug | Level::Trace => "debug",
            };
            eprintln!("render-server {level} [{}] {}", record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

pub fn install() {
    let level = match std::env::var("SOLIDCRAFT_RENDER_LOG").ok().as_deref().map(str::trim) {
        Some("error") => LevelFilter::Error,
        Some("info") => LevelFilter::Info,
        Some("debug") => LevelFilter::Debug,
        _ => LevelFilter::Warn,
    };
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(level);
    }
}
