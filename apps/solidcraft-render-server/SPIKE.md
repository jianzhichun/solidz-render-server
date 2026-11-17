# solidcraft-render-server: spike on the build hosts (M1, M2, M3)

What this is: upstream's editing UI (`SolveApp`, `embed_editor` on) with no window, drawn offscreen
into a texture, streamed over leg A to the host that started it (PROTOCOL.md). The design and its
reasons: the host plugin's design notes (`docs/stream/design.md`).

**State when this file was written: nothing here has been compiled or run.** The code was written
on a machine where cargo is forbidden, from the sources of v0.42.0, egui/eframe/egui-winit/epaint
0.37.0, wgpu 30.0.1 and the vendored egui-wgpu, with the defining `file:line` next to every
non-obvious call. The compile phase starts with "Build" below; the most likely compile breaks are
listed at the end.

## Files

| Path | What |
|---|---|
| `src/main.rs`, `src/cli.rs` | composition root, flags |
| `src/domain/` | no I/O, no GPU, unit-tested: `wire` (messages), `envelope` (framing), `input` (wire event → `egui::Event`, IME included), `geometry` (CSS px → device px, 4096 cap), `damage` (64² tiles → rectangles), `codec` (raw / JPEG stripes / PNG), `pacing` (credits, 60 passes/s, on demand), `samples` (scene MSAA by dpr), `memory` (device memory by label) |
| `src/adapters/` | `gpu` (headless device + `RenderState`, what it holds by label), `painter` (one frame into the offscreen texture, egui-wgpu's own call order), `readback`, `pictures` (the model's pictures: small ones on the CPU, larger ones on a picture renderer made on demand and dropped when idle), `encode` (JPEG/PNG with `image`, parallel with rayon), `link` (leg A: reader/writer threads), `control` (upstream's control server, no token or discovery files), `clipboard` (arboard), `logging` |
| `src/app/` | `ui_host` (SolveApp assembly, one pass, paint, screenshots), `frames` (readback → diff → encode against credits, idle refinement), `output` (cursor/IME/copy), `services` (pickers → `ask_save`/`ask_open`), `policy` (read-only backstop), `stream` (the leg-A loop), `snapshot` (M1, offline `--script`) |
| `spike/leg-a.mjs` | Node harness that plays the host on leg A (M2 over the wire, M3 server+transport latency) |
| `spike/ctl.mjs` | one control-channel request from a shell |
| `spike/mem.sh` | memory per scenario on Linux (M1 run + leg-A orbit, dpr 1 and 2; see "Memory") |
| `spike/userdirs.sh` | what one session writes under the user's own folders (Linux, fresh `HOME`; see "Behaviour to know while measuring") |
| `spike/m2-orbit.json` | offline M2 script: right-drag 200 px in the pane centre (default "Fusion" navigation: right drag orbits, `crates/ui-egui/src/prefs.rs:120-129`) |

Edits to existing upstream files (design §1.1, the whole list):
- E1: `crates/ui-egui/src/control_protocol.rs` (new, `AppProtocol` moved unchanged from
  `apps/solidcraft/src/control_server.rs`), its `pub mod` in `crates/ui-egui/src/lib.rs` (native only),
  and the desktop's `control_server.rs` using it.
- E2: `crates/engine/src/read_only.rs` (new, process-wide reason), `pub mod read_only;` and two hunks
  in `Session::execute` (`crates/engine/src/lib.rs`), and `crates/engine/tests/read_only.rs` (its own
  test binary: the switch is process-wide). The second hunk sits before the crash journal takes the
  command's outcome (`crash_outcome`), so a refused change is journalled as failed, never replayed.
- E3 (bookkeeping, not in the design's table): one row in `xtask/src/layers.rs` `TABLE` registering
  `render-server` as an app (`Class::Exempt`, like `cli` and `web`); without it `cargo xtask layers`
  (part of `cargo xtask ci`) fails with "unknown workspace crate".
- E4 (memory): `crates/ui-egui/src/fonts.rs` keeps the system fallback faces as shared bytes
  (`egui::epaint::text::Blob`) and installs them with `FontData::from_blob`, instead of copying each
  file into every installation: one copy of each face instead of two (more while a font change is
  being applied). These files are tens of MB on macOS and Windows. Upstreamable as it is.
- E5 (per-user folders, `--config-dir`): `crates/engine/src/recovery.rs` `set_user_dir` (a
  process-wide folder that `default_dir`, `data_dir` and `cache_dir` answer with, so crash reports
  follow) and `crates/engine/tests/user_dir.rs` (its own test binary). Upstreamable as it is.
- E6 (the UI's language): `crates/ui-egui/src/i18n.rs` `tr_engine_error` (a command refused in its
  context, a missing parameter), `dialogs.rs` (`shown_error` through it, Create Issue… translated,
  no Create Issue for the host's read-only refusal: that check uses E2), `toolbar.rs` (Export itself
  from command search asks where to save; `EXPORT_EXTENSIONS`, also in `context_menu.rs`),
  `dialogs_sheet.rs` (the flat-pattern export's status lines), a test in `file_picker.rs`, and the
  messages in `assets/locales/pending/command-refusals-and-export-status.json`. Upstreamable except
  the read-only check.
- The workspace already takes `apps/*` (`Cargo.toml:3`); `Cargo.lock` gains the package on the first
  build (not edited by hand).

## Build (every host)

The toolchain comes from `rust-toolchain.toml` (1.99.1). From a checkout of this repository:

```sh
cargo build -p solidcraft-render-server --release
# The first build on this branch adds the package to Cargo.lock: commit that change (git add Cargo.lock).
cargo test  -p solidcraft-render-server                 # domain + encode + services + cli: no GPU needed
cargo test  -p solidcraft-engine --lib read_only        # E2 rules
cargo test  -p solidcraft-engine --test read_only       # E2 through Session::execute (own process)
cargo clippy -p solidcraft-render-server --all-targets
cargo fmt -p solidcraft-render-server                   # never run on the writing machine: format, then commit
cargo test  -p solidcraft-ui-egui i18n                  # the catalog guard reads every UI source, control_protocol.rs too
cargo xtask layers                                      # E3: the new app is registered
# E1 left the desktop app as it was (needs the desktop's system libraries; Mac or Windows host):
cargo test  -p solidcraft --bin solidcraft control_server
cargo check -p solidcraft-ui-egui --target wasm32-unknown-unknown   # the cfg gate keeps the web build
```

Binary: `target/release/solidcraft-render-server` (`.exe` on Windows). Linux needs no X11/Wayland
libraries for it (arboard without `wayland-data-control`, no rfd, no winit).

Design for M1/M2: the 0.2.0 acceptance fan, a saved design with a lofted, patterned body (any `.sc` / `.solidz` of a few MB)
(copy it to the host), and the empty design (no `--open`). A `.solidz` file is opened the same way if
upstream's open accepts it; if not, copy it to `.sc`.

## Graphics environment

| Host | Environment | Flag |
|---|---|---|
| Linux, no display, software Vulkan (lavapipe) | `sudo apt install mesa-vulkan-drivers libvulkan1 vulkan-tools`; `env -u DISPLAY -u WAYLAND_DISPLAY VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json WGPU_BACKEND=vulkan` (check: `vulkaninfo --summary` lists `llvmpipe`) | `--adapter software` |
| Linux with a GPU | `env -u DISPLAY -u WAYLAND_DISPLAY WGPU_BACKEND=vulkan` | `--adapter hardware` |
| Apple-silicon Mac | none (Metal; no NSApplication or window is needed) | `--adapter hardware` |
| Windows | none (D3D12 first, then Vulkan); `WGPU_BACKEND=vulkan` to compare | `--adapter hardware` |

`SOLIDCRAFT_RENDER_LOG=info` prints the adapter, depth and scene MSAA chosen (`graphics: …`).
`--adapter auto` (the default) takes the best hardware adapter, else a software one.

## M1: offscreen frames of chrome + model

```sh
BIN=target/release/solidcraft-render-server
export SOLIDCRAFT_CONTROL_TOKEN=$(openssl rand -hex 32)     # else one is generated and printed as `control-token …`
$BIN --snapshot m1-dpr2 --frames 30 --open fan.sc --size 1280x800 --dpr 2 --hold-ms 120000 2> m1-dpr2.log &
until grep -q 'frames done' m1-dpr2.log; do sleep 1; done
PORT=$(sed -n 's/^control 127.0.0.1:\([0-9]*\)$/\1/p' m1-dpr2.log)
node spike/ctl.mjs $PORT $SOLIDCRAFT_CONTROL_TOKEN ui.handles '{"prefix":"tab:"}'
node spike/ctl.mjs $PORT $SOLIDCRAFT_CONTROL_TOKEN ui.handles '{"prefix":"row:"}'
node spike/ctl.mjs $PORT $SOLIDCRAFT_CONTROL_TOKEN ui.inspect
node spike/ctl.mjs $PORT $SOLIDCRAFT_CONTROL_TOKEN document.inspect
node spike/ctl.mjs $PORT $SOLIDCRAFT_CONTROL_TOKEN ui.worldToScreen '{"points":[[0,0,0]]}'
node spike/ctl.mjs $PORT $SOLIDCRAFT_CONTROL_TOKEN ui.screenshot '{"path":"'$PWD'/m1-dpr2/shot.png"}'
wait
```

Then the same with `--dpr 1` (directory `m1-dpr1`). Windows (PowerShell): `$env:SOLIDCRAFT_CONTROL_TOKEN = …;
Start-Process .\target\release\solidcraft-render-server.exe -ArgumentList '--snapshot','m1-dpr2',… -RedirectStandardError m1-dpr2.log`.

Output: `m1-dpr2/frame-0001.png … frame-0030.png` (2560×1600) and `m1-dpr2/frames.json`
(`adapter`, `backend`, `software`, `samplesRequested`, `samples` in effect, `size`, per frame `timing`
{pass, paint, readback, encode} ms and the design `revision`). Handle and world-to-screen points are
UI points = CSS px: multiply by the dpr for the PNG pixel.

Pass when all hold (design §5):
1. exit 0 and no window: on Linux `DISPLAY` and `WAYLAND_DISPLAY` were unset; on the Mac
   `lsappinfo list | grep -i render-server` finds nothing and no window of the pid shows in Mission
   Control; on Windows `(Get-Process -Id $pid).MainWindowHandle` is 0 while it holds.
2. frame 30: `ui.handles` `tab:` gives ≥ 5 ribbon tabs and `row:` a browser row per body, all inside
   the frame, and the PNG pixel at each handle differs from the background; `ui.inspect`'s viewport
   rect is smaller than the frame (chrome around it). A Pillow check:
   `python3 -c "from PIL import Image; im=Image.open('m1-dpr2/frame-0030.png'); print(im.getpixel((int(X*2),int(Y*2))), im.getpixel((5,5)))"`.
3. model: each body's bbox centre from `document.inspect` → `ui.worldToScreen` → that pixel differs
   from the viewport background for ≥ 80 % of the bodies.
4. `frames.json`: a hardware adapter (`software: false`) on the Mac and Windows, `samples` 4 (or the
   reason it is 1 in the log), `timing` on every frame.
5. a person looks at frames 1 and 30 (ribbon, browser, timeline, shaded model, Chinese labels) and
   lists every visible upstream product name (a finding, not a gate). Frame 1 may still lack system
   fonts (they load in the background); frame 30 must not.

## M2: scripted input, asserted by pixels and revision

Offline, the same wire events through the same translation (`src/domain/input.rs`), no host:

```sh
$BIN --snapshot m2-orbit --frames 60 --gap-ms 33 --open fan.sc --size 1280x800 --dpr 1 \
     --script spike/m2-orbit.json --hold-ms 30000 2> m2.log &
```

Pass: `frames.json` revision the same on every frame; frame 4 and frame 45 differ in ≥ 50 % of the
viewport's pixels; `ui.camera` during the hold differs from a run without `--script` by > 10° of yaw.

Edit case (the revision must move): `--script spike/m2-edit.json --frames 30 --gap-ms 50` clicks the
fan's `row:Body1` (125, 294 at 1280x800, from `ui.handles`) and presses Delete at frame 10.

Over leg A (the harness plays the host: hello/token check, policy, resize, visible, acks every frame,
answers asks with `null`, then `stop`):

```sh
node spike/leg-a.mjs --bin $BIN --open fan.sc --case orbit --size 1280x800 --dpr 1 --out m2-frames
```

Pass: exit 0 after `bye stop`; `orbit.after.camera` yaw − `orbit.before.camera` yaw > 10° (radians in
the reply); `orbit.*.revision` equal; `drag.frames` > 0.

`--case edit` selects the first body by its browser row and presses Delete: `edit.after.revision` moves
and the body count drops. With `--policy REASON` (read-only from the start) the revision and the body
count stay, and `edit.status` shows the refusal with the reason.

The other M2 cases of design §5 (ribbon tool + Enter, typing a dimension, Chinese IME, clipboard,
cursor over a splitter, one Session with the MCP bridge, read-only, trial export, UI save) need
coordinates from `ui.handles` of the running UI and, for the last four, the plugin's Node side
(WP-N). Write them as `--script` steps (events at frames) or extend `leg-a.mjs`'s `runCase`. The
IME sequence for a focused rename box, as the pane sends it:

```json
[{"frame": 10, "events": [{"e": "preedit", "text": "ce", "range": null}]},
 {"frame": 12, "events": [{"e": "preedit", "text": "测", "range": [0, 1]}]},
 {"frame": 14, "events": [{"e": "commit", "text": "测试"}]},
 {"frame": 16, "events": [{"e": "key", "key": "Enter", "physical": "Enter", "pressed": true, "repeat": false,
                           "mods": {"alt": false, "ctrl": false, "shift": false, "meta": false}}]}]
```

Pass for IME: the body is named 测试 (`document.inspect`), the field's pixels change at frames 11
and 13, and over leg A an `ime` message with a caret box arrived.

## M3: cost and latency

Server stages per frame: `frames.json` `timing` (snapshot) and `frame.timing` (leg A: pass, paint,
readback, diff, encode). Server + transport latency with no webview, at R2 and R1:

```sh
node spike/leg-a.mjs --bin $BIN --open fan.sc --case orbit --size 1280x800 --dpr 1 --moves 120 --hz 120   # R2
node spike/leg-a.mjs --bin $BIN --open fan.sc --case orbit --size 1280x800 --dpr 2 --moves 120 --hz 120   # R1
node spike/leg-a.mjs --bin $BIN --open fan.sc --case idle  --idle-ms 60000                                 # idle
node spike/leg-a.mjs --bin $BIN --open fan.sc --case orbit,click,typing,idle --dpr 1 --moves 120 --hz 120   # all in one run
```

`click` (a ribbon button from `ui.handles`, timed from the release, then Escape) and `typing` (the
command search field: keys 2 and 5, then 测试 as pinyin preedit/commit) report per input the latency
and that frame's stages plus `rest` (waiting for the pass to start + loopback); `drag.rows` does the
same per drag frame (the first frame of each `inputSeq`; later ones with the same `inputSeq` are the
UI's own repaints, counted as `drag.after`); `idle.slices` gives frames and CPU per 5 s. On a host
with a CPU quota and no GPU, set `LP_NUM_THREADS` and `RAYON_NUM_THREADS` to the quota: lavapipe
otherwise starts one thread per host core and the quota throttles the whole process (on a Linux build host with a 6-core quota,
6 of 20 cores: R2 orbit 133 → 44 ms p50, 7 → 26 frames/s).

Read from the summary: `drag.latencyMs` p50/p95 (input sent → first frame carrying that
`inputSeq` received), `drag.fps`, `stageMsP50`, `bytes`, `startupMs.firstFrame` (cold start),
`idleFrames`. CPU and memory while it runs: `ps -o %cpu=,rss= -p PID` (Mac/Linux), Task Manager or
`Get-Process -Id PID | Select CPU,WS` (Windows). End-to-end in the real pane (decode + draw + vsync)
is the pane's probe mode (WP-P), not this harness.

Go / no-go (design §5): orbit input → pixel p50 ≤ 60 ms, p95 ≤ 100 ms, ≥ 30 frames/s at R2 on both,
and at R1 on the Mac (no-go: p50 > 90 ms or < 20 frames/s at R2 after turbojpeg + dirty rectangles);
typing p50 ≤ 50 ms, p95 ≤ 80 ms; click → response p50 ≤ 60 ms; idle 0 frames and ≤ 1 % CPU over 60 s;
cold start ≤ 3 s p50 (Mac, hardware); RSS per process with the fan recorded, ≤ 600 MB to keep 3 live.
The harness's numbers are the server + loopback share: if they already miss the budget, the pane
cannot make it up. Levers in order: smaller rectangles, the JPEG encoder (`turbojpeg`), a pipelined
readback ring, the half-size draft during drags, then H.264.

## Memory

Where it goes: `frames.json` `gpuMemory` (M1) and the `gpu memory:` line at `stop` (leg A,
`SOLIDCRAFT_RENDER_LOG=info`) list what the device holds, by label, where the backend can say (Vulkan,
D3D12; on a Mac read `footprint PID`, which counts Metal's memory too). On lavapipe the device's memory
is process memory (`RssShmem`), so on Linux `spike/mem.sh` splits a scenario into malloc (`RssAnon`)
and graphics (`RssShmem`): the M1 run (read 3 s into the hold) and a leg-A orbit, at dpr 1 and 2.

What a streamed frame does not need, and what was cut (all in this crate except E4):

| Cut | Why it was there | Effect |
|---|---|---|
| Scene MSAA by dpr (`domain/samples.rs`, `--msaa`): 1 at dpr ≥ 2, else 2 where the device has it | the desktop's 4× | 4× colour + depth were 188 of the scene's 203 MiB at 2560×1600 |
| Small pictures on the CPU rasterizer, larger ones (and analysis views) on a picture renderer made on demand and dropped 3 s after (`adapters/pictures.rs`) | the desktop makes a second copy of the viewport's pipelines at start | on lavapipe ~140 MiB of malloc (compiled shaders) |
| One frame on the CPU: compared where it is mapped, changed rectangles copied over the last frame sent (`adapters/readback.rs`, `domain/damage.rs`) | a read-back copy and the last frame sent | one frame (4 / 16 MiB at dpr 1 / 2) and its copy per frame |
| The screenshot read-back buffer is dropped after each screenshot (`app/ui_host.rs`) | kept after the first one | one frame of GPU-visible memory |
| Small device memory blocks (`MemoryHints::MemoryUsage`, `adapters/gpu.rs`) | 128–256 MiB blocks | reserved 192 → 92 MiB on Vulkan; no RSS change on lavapipe |
| E4: system fallback faces shared, not copied into each font installation | — | one copy of each face instead of two (tens of MB on macOS/Windows; 4 MB on the Linux build host) |

Measured on a Linux build host (lavapipe, 6 threads), the fan at 1280×800, `spike/mem.sh`: MiB of
VmRSS, which includes ~170 MiB of the binary's own pages (a Mac's footprint does not count clean file
pages); malloc / graphics (RssAnon / RssShmem) in brackets. "orbit" is the highest sample of the leg-A
orbit run.

| Build | M1 dpr 1 | orbit dpr 1 | M1 dpr 2 | orbit dpr 2 |
|---|---|---|---|---|
| before | 724 (478 / 75) | 736 | 918 (496 / 251) | 967 |
| scene MSAA 1 alone | 671 | 697 | 758 | 808 |
| no picture renderer alone | 573 | 595 | 766 | 821 |
| small memory blocks alone | 714 | 737 | 919 | 968 |
| all cuts, `--msaa 4` | 568 (331 / 68) | 580 | 761 (344 / 246) | 811 |
| all cuts (MSAA by dpr: 1 on lavapipe at both) | **521** (322 / 29) | **538** | **601** (341 / 91) | **642** |

The frames themselves differ only where MSAA was: frame 30 of the M1 run differs from the 4× one in
0.9 % of its pixels at dpr 1 and 0.5 % at dpr 2 (by more than 24 levels), all on model edges, and not
at all from a run with only `--msaa 1`. At dpr 1 one sample shows stairs on the model's edges when
looked at closely, hence 2× there on devices that have it (lavapipe has 1 and 4 only, so it draws 1);
at dpr 2 they do not show at 1:1.

Latency did not move (leg-A orbit, 3 cold starts each, p50 / p95 ms and frames/s, same host, 6
threads): before, R2 42.9–45.4 / 49–51 at 25 fps and R1 118–123 / 137–215 at 8.6–9.0; all cuts at
`--msaa 4` (4 runs), R2 43.0–45.8 / 47–49 at 25–26 and R1 117–124 / 120–203 at 8.6–9.0; all cuts (one sample
on lavapipe), R2 30.9–32.2 / 36–39 at 36–37 and R1 77–80 / 88–145 at 13.7–14.1 (lavapipe draws
fewer samples faster; a GPU will not gain much there).

A large picture (the model's 1280×800 `ui.render`) still makes the picture renderer: on lavapipe
malloc +143 and device memory +54 MiB; dropped 3 s later, malloc fell back by 39 only and the device
memory block stayed (glibc and the Vulkan allocator keep what they freed; Metal frees its objects).
The reply's `renderer` says `gpu`/`gpu-software`; a 320×200 one says `cpu` and costs nothing.
A small picture of a view a surface analysis shades still goes to the GPU (the rasterizer draws no
analysis colours). As upstream's, a picture the GPU cannot draw (the picture renderer cannot be made
on the device, or the device is lost) is drawn by the CPU rasterizer instead of failing.
The engine logs its CPU warning after every picture the CPU draws ("no GPU adapter qualified, or
SOLIDCRAFT_GPU=cpu"); after the small ones drawn there by choice that is wrong, so the log leaves it
out (one `debug` line says what was drawn instead); it stays for the fallbacks above.

With no design open the same process holds 356 / 417 MiB (malloc 219 / 220): the fan's design,
history and meshes are ~105 MiB of malloc.

What stays: the scene's own colour and depth at the frame's size (the depth is 32-bit float with
stencil, 8 bytes a pixel, for reversed depth and section caps), the frame texture and its read-back
buffer, and the last frame sent. Not measured on a Mac yet (no GPU on the build hosts): the Mac's
footprint must be read again there.

## Behaviour to know while measuring

- A device that does not open (`--adapter hardware` on a host with only lavapipe, say): no message
  on leg A, the reason on stderr (`render-server error …`), exit 1. The host takes that as an exit
  before `ready` and puts the session on the viewer (a `bye` there would count as a crash). A panic
  in the frame loop: `bye error` with the panic text, exit 4 (exit codes, for logs: 0 clean, 1 setup,
  2 protocol, 3 device lost, 4 panic).
- Passes run on demand: input, a host message, the UI's own repaint request (control requests wake
  it through the context's repaint callback), at most 60 a second. No frame when nothing changed,
  when the pane is hidden, or with no credit (2 at start, one per `ack`).
- Rectangles ≤ 65 536 px go raw; larger ones as JPEG q80 in stripes of ≤ 256 rows, encoded in
  parallel. 200 ms after the last JPEG frame (and no input), the JPEG-covered region goes again as
  PNG from the true pixels (the `image` JPEG encoder is 4:2:2 only, so no q95 4:4:4 refinement).
- `ui.screenshot` is answered from the offscreen frame, also while the pane is hidden (the UI is
  never told it is minimised or occluded).
- Viewport commands (close, minimise, title, cursor warp, `ui.resize`'s inner size) are ignored: the
  pane decides the size. `OpenUrl` is not opened.
- Read-only (`policy`): the engine hook refuses and undoes changes; the backstop re-imports the kept
  design if the revision still moved, and keeps another design that was opened instead.
- `ask_done.ok` = an answer path was given and the pass that ran the picker's completion set no
  error on the status line (the UI reports export failures there).
- The user's own folders: `spike/userdirs.sh BIN DESIGN OUT [args]` runs one snapshot session with
  an edit under a fresh `HOME` and lists what it made there. Linux, lavapipe, the fan: without
  `--config-dir`, `~/.cache/solidcraft/thumbnails/<hash>.png` (Home's picture of the opened design,
  upstream's per-user cache, which a desktop install shares) besides the driver's own
  `~/.cache/mesa_shader_cache_db`; with `--config-dir DIR`, the picture is `DIR/thumbnails/<hash>.png`
  and `HOME` holds only the driver's cache. No preferences or keyboard-shortcut file is written
  either way: there is no settings store here (PROTOCOL.md, "Starting").

## Most likely compile breaks (check these first)

- `adapters/gpu.rs`: the two closures assigned to `native_adapter_selector` and `device_descriptor`
  rely on closure signature deduction through `Arc::new` (the desktop does the same for
  `device_descriptor`, `apps/solidcraft/src/main.rs:1641`); if the selector's fails, annotate
  `|adapters: &[wgpu::Adapter], _surface: Option<&wgpu::Surface<'_>>|`.
- `adapters/painter.rs`: `rs.renderer.read().frame_compositor.clone()` (an
  `Option<Arc<dyn FrameCompositor>>`, `vendor/egui-wgpu/src/renderer.rs:300`) and
  `DeviceLost::survive` around a closure that borrows `deltas` mutably (`lib.rs:209`).
- `adapters/readback.rs`: `wgpu::PollType::Wait { submission_index, timeout }` (wgpu-types 30
  `lib.rs:176`) and `get_mapped_range()` returning a `Result` (`wgpu-30.0.1/src/api/buffer.rs:620`).
- `app/ui_host.rs`: the `RawInput` / `ViewportInfo` struct literals (egui 0.37 field names), the
  `FullOutput` destructuring (all five fields), `viewport_output.remove(&ViewportId::ROOT)` (a
  `BTreeMap`, `viewport.rs:179`), `ctx.global_style()` (`context.rs:2599`), `app.preferences.msaa` as
  `u32`, `app.status` as `Option<(String, f64, bool)>` (`lib.rs:761`).
- `domain/wire.rs`: `#[serde(tag = "k", rename_all = "snake_case", rename_all_fields = "camelCase")]`
  (serde ≥ 1.0.183; the lock has 1.0.229).
- `app/output.rs`: `egui::output::IMEOutput` and `egui::IMEPurpose` paths.
- Clippy may ask for `#[allow(clippy::too_many_arguments)]`-style changes; the workspace allows that
  one already.
