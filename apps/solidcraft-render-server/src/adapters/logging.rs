//! `log` records to stderr, one line each (the host keeps stderr in its engine log). No logger
//! crate is in the tree, and this one needs no more. `SOLIDCRAFT_RENDER_LOG=error|warn|info|debug`
//! (default `warn`; wgpu's own `info` lines are many). The engine's warning after each picture
//! the CPU draws ("no GPU adapter qualified") is left out after the small ones drawn there by
//! choice ([`crate::adapters::pictures::cpu_by_choice`]); it stays when a GPU really is missing.

use log::{Level, LevelFilter, Log, Metadata, Record};

struct Stderr;

static LOGGER: Stderr = Stderr;

impl Log for Stderr {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) && !misleading(record.target(), super::pictures::cpu_by_choice(), || record.args().to_string()) {
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

/// The engine's CPU warning (`solidcraft_engine::capture::CPU_WARNING`) after a picture the CPU
/// drew by choice: it would say no GPU qualified while one draws the UI.
fn misleading(target: &str, cpu_by_choice: bool, text: impl FnOnce() -> String) -> bool {
    cpu_by_choice && target == "solidcraft_engine::capture" && text() == solidcraft_engine::capture::CPU_WARNING
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

#[cfg(test)]
mod tests {
    use super::*;
    use solidcraft_engine::capture::CPU_WARNING;

    /// Only the engine's CPU warning, and only after a picture drawn there by choice, is left out.
    #[test]
    fn the_cpu_warning_is_left_out_only_after_a_picture_drawn_there_by_choice() {
        let target = "solidcraft_engine::capture";
        assert!(misleading(target, true, || CPU_WARNING.into()));
        assert!(!misleading(target, false, || CPU_WARNING.into()), "no GPU: the warning stays");
        assert!(!misleading(target, true, || "another warning".into()));
        assert!(!misleading("solidcraft_render_server", true, || CPU_WARNING.into()));
    }
}
