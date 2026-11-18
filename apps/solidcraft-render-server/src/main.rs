//! solidcraft-render-server: SolidCraft's editing UI with no window. Each frame is drawn
//! offscreen and streamed to the embedding host that started this process; the host sends the
//! user's pointer, keyboard and IME input back; the model drives the same session over the
//! control channel. Protocol: PROTOCOL.md beside this crate; how to build, run and measure it:
//! SPIKE.md.
//!
//! Layers: `domain` (rules, no I/O), `adapters` (GPU, sockets, encoders, clipboard), `app` (the
//! UI host and the two modes), and this composition root.

mod adapters;
mod app;
mod cli;
mod domain;

use app::snapshot::{self, SnapshotArgs};
use app::stream::{self, StreamArgs};
use app::ui_host::UiOptions;
use cli::{Mode, Parsed};
use domain::geometry::FrameSize;

/// The product this editor is part of: the UI's messages name it where they name SolidCraft, and
/// SolidCraft's own report, update and community entries are left out
/// (`solidcraft_ui_egui::brand`).
const PRODUCT: &str = "SolidZ";

/// Why `diagnostics.report_issue` refuses here (shown as "<command>当前不可用：<this>").
const REPORTS_REFUSED: &str = "这里不能发送问题报告";

fn main() {
    // Before any message is read.
    solidcraft_ui_text::set_product_name(PRODUCT);
    // SolidCraft's own problem reports (system info, the log, a screenshot to its ticket site) never
    // leave from here, whoever asks: the user's UI or the model over the control channel.
    solidcraft_engine::report_issue::refuse(Some(REPORTS_REFUSED));
    adapters::logging::install();
    let options = match cli::parse(std::env::args().skip(1)) {
        Ok(Parsed::Run(o)) => *o,
        Ok(Parsed::Help) => {
            eprintln!("{}", cli::USAGE);
            return;
        }
        Ok(Parsed::Version) => {
            eprintln!("solidcraft-render-server {} (protocol {})", env!("CARGO_PKG_VERSION"), domain::wire::PROTOCOL_VERSION);
            return;
        }
        Err(e) => {
            eprintln!("solidcraft-render-server: {e}\n{}", cli::USAGE);
            std::process::exit(stream::EXIT_SETUP);
        }
    };
    if let Some(session) = &options.session {
        log::info!("session {session}");
    }
    // Before the UI reads any of them: recovery, crash reports, data and cache in the host's folder.
    if let Some(dir) = options.config_dir {
        solidcraft_engine::recovery::set_user_dir(dir);
    }
    let ui = UiOptions { lang: options.lang, dark: options.dark };
    let size = FrameSize::new(options.css.0, options.css.1, options.dpr);
    let code = match options.mode {
        Mode::Stream(addr) => stream::run(StreamArgs { addr, ui, size, adapter: options.adapter, msaa: options.msaa }),
        Mode::Snapshot { dir, frames, open, script, hold, gap } => {
            snapshot::run(SnapshotArgs { dir, frames, open, script, hold, gap, ui, size, adapter: options.adapter, msaa: options.msaa })
        }
    };
    std::process::exit(code);
}
