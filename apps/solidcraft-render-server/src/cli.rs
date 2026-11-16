//! The command line (PROTOCOL.md, "Starting (leg A)"; the host's copy is `RENDER_SERVER_FLAGS`
//! in contract.ts).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use solidcraft_viewport_gpu::select::Policy;

use crate::domain::samples::Msaa;

pub const USAGE: &str = "usage:
  solidcraft-render-server --stream 127.0.0.1:PORT [--session ID] [--size WxH] [--dpr N] [--lang zh] [--theme dark|light] [--adapter auto|hardware|software] [--msaa auto|1|2|4]
  solidcraft-render-server --snapshot DIR --frames N [--open FILE] [--script FILE] [--hold-ms N] [--gap-ms N] [--size WxH] [--dpr N] [--lang zh] [--theme dark|light] [--adapter ...] [--msaa ...]
environment: SOLIDCRAFT_RENDER_TOKEN and SOLIDCRAFT_CONTROL_TOKEN (64 hex each; stream mode), SOLIDCRAFT_RENDER_LOG=error|warn|info|debug";

#[derive(Debug, PartialEq)]
pub enum Mode {
    /// Leg A to the host's listener (loopback only).
    Stream(SocketAddr),
    /// M1: `frames` PNGs into `dir`, then the control channel for `hold`.
    Snapshot { dir: PathBuf, frames: u32, open: Option<PathBuf>, script: Option<PathBuf>, hold: Duration, gap: Duration },
}

#[derive(Debug, PartialEq)]
pub struct Options {
    pub mode: Mode,
    /// For log lines only.
    pub session: Option<String>,
    /// The pane, CSS px.
    pub css: (u32, u32),
    pub dpr: f32,
    pub lang: String,
    pub dark: bool,
    pub adapter: Policy,
    /// The scene's samples (`domain::samples`).
    pub msaa: Msaa,
}

#[derive(Debug, PartialEq)]
pub enum Parsed {
    Run(Options),
    Help,
    Version,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Parsed, String> {
    let mut args = args.into_iter();
    let (mut stream, mut snapshot, mut frames, mut open, mut script) = (None, None, None, None, None);
    let (mut hold, mut gap) = (Duration::ZERO, Duration::from_millis(50));
    let mut o = Options {
        mode: Mode::Stream(SocketAddr::from(([127, 0, 0, 1], 0))),
        session: None,
        css: (1280, 800),
        dpr: 1.0,
        lang: "zh".into(),
        dark: true,
        adapter: Policy::Auto,
        msaa: Msaa::Auto,
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--help" | "-h" => return Ok(Parsed::Help),
            "--version" | "-V" => return Ok(Parsed::Version),
            "--stream" => {
                let addr: SocketAddr = value()?.parse().map_err(|e| format!("--stream: {e}"))?;
                if !addr.ip().is_loopback() {
                    return Err(format!("--stream: {addr} is not a loopback address"));
                }
                stream = Some(addr);
            }
            "--session" => o.session = Some(value()?),
            "--size" => o.css = size(&value()?)?,
            "--dpr" => o.dpr = value()?.parse::<f32>().ok().filter(|d| d.is_finite() && *d > 0.0).ok_or("--dpr: a positive number")?,
            "--lang" => o.lang = value()?,
            "--theme" => {
                o.dark = match value()?.as_str() {
                    "dark" => true,
                    "light" => false,
                    other => return Err(format!("--theme: dark or light, not {other}")),
                }
            }
            "--adapter" => {
                o.adapter = Policy::parse(&value()?)
                    .filter(|p| *p != Policy::Cpu)
                    .ok_or("--adapter: auto, hardware or software (the UI needs a GPU device)")?
            }
            "--msaa" => o.msaa = Msaa::parse(&value()?).ok_or("--msaa: auto, 1, 2, 4 or 8")?,
            "--snapshot" => snapshot = Some(PathBuf::from(value()?)),
            "--frames" => frames = Some(value()?.parse::<u32>().ok().filter(|n| *n > 0).ok_or("--frames: a positive count")?),
            "--open" => open = Some(PathBuf::from(value()?)),
            "--script" => script = Some(PathBuf::from(value()?)),
            "--hold-ms" => hold = Duration::from_millis(value()?.parse().map_err(|_| "--hold-ms: milliseconds")?),
            "--gap-ms" => gap = Duration::from_millis(value()?.parse().map_err(|_| "--gap-ms: milliseconds")?),
            other => return Err(format!("unknown argument {other}")),
        }
    }
    o.mode = match (stream, snapshot) {
        (Some(addr), None) => Mode::Stream(addr),
        (None, Some(dir)) => Mode::Snapshot { dir, frames: frames.unwrap_or(30), open, script, hold, gap },
        (Some(_), Some(_)) => return Err("--stream and --snapshot exclude each other".into()),
        (None, None) => return Err("--stream or --snapshot is required".into()),
    };
    Ok(Parsed::Run(o))
}

/// `WxH` in CSS px.
fn size(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("--size: WxH, not {s}");
    let (w, h) = s.split_once(['x', 'X']).ok_or_else(bad)?;
    let (w, h) = (w.trim().parse::<u32>().map_err(|_| bad())?, h.trim().parse::<u32>().map_err(|_| bad())?);
    if w == 0 || h == 0 {
        return Err(bad());
    }
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &str) -> Result<Options, String> {
        match parse(args.split_whitespace().map(String::from))? {
            Parsed::Run(o) => Ok(o),
            other => Err(format!("{other:?}")),
        }
    }

    #[test]
    fn the_hosts_command_line() {
        let o = run("--stream 127.0.0.1:4100 --session s1 --size 1280x800 --dpr 2 --lang zh --theme light").unwrap();
        assert_eq!(o.mode, Mode::Stream("127.0.0.1:4100".parse().unwrap()));
        assert_eq!((o.css, o.dpr, o.dark, o.session.as_deref(), o.msaa), ((1280, 800), 2.0, false, Some("s1"), Msaa::Auto));
        assert_eq!(run("--stream 127.0.0.1:1 --msaa 4").unwrap().msaa, Msaa::Samples(4));
        assert!(run("--stream 127.0.0.1:1 --msaa 3").is_err());
    }

    #[test]
    fn snapshot_defaults_and_refusals() {
        let o = run("--snapshot out --open fan.sc").unwrap();
        assert!(matches!(o.mode, Mode::Snapshot { frames: 30, open: Some(_), .. }));
        assert!(run("--stream 10.0.0.2:4100").unwrap_err().contains("loopback"));
        assert!(run("--size 0x10 --snapshot out").is_err());
        assert!(run("--adapter cpu --snapshot out").is_err());
        assert!(run("--session s").unwrap_err().contains("required"));
        assert!(run("--stream 127.0.0.1:1 --snapshot out").is_err());
    }
}
