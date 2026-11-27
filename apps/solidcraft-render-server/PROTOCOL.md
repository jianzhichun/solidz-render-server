# solidcraft-render-server wire protocol, version 1

The render server runs the editing UI (`SolveApp`, `embed_editor` on) with no window, draws each frame
into an offscreen texture, and streams it to an embedding host. The host sends the user's pointer,
keyboard and IME input back as UI input. The design file and the exports stay the host's.

This file is the Rust side's copy of the protocol. The embedding host (SolidZ) keeps the same protocol
as TypeScript types; the two must agree field by field, and `v` moves with any change an old peer would misread.

## Legs and framing

| Leg | Between | Transport | Framing |
|---|---|---|---|
| A | render server and host process | one loopback TCP connection; the render server connects to `--stream 127.0.0.1:PORT` (a listener the host made for this one process) | `u32 LE length` + envelope, repeated |
| B | host process and the page | one WebSocket (binary messages) on the host's own origin | one WebSocket message = one envelope |

**Envelope** = `u32 LE header_len` + `header_len` bytes of UTF-8 JSON (an object with `"k"`, the kind)
+ payload bytes (the rest of the envelope). Only `frame` has a payload; every other kind has none.

Limits: header at most 1 MiB (`1 << 20`); envelope at most 64 MiB (`64 << 20`). A peer that receives
more, a header that is not a JSON object, a kind it does not know for that direction, or `v` other
than its own sends `error` and closes the connection.

Messages on one leg arrive in the order they were sent. The host relays to the page exactly
`frame`, `cursor`, `ime`, `copied` (unchanged, payload included), and to the render server exactly
the page's `input`, `resize`, `visible`, `ack` (unchanged). Everything else is the host's own.

## Units

- Positions, sizes and rectangles exchanged with the UI: **CSS pixels of the page's pane = egui
  points**. The render server runs the context at zoom 1 with `pixels_per_point = dpr`, so
  `RawInput.screen_rect = Rect(0, 0, w, h)` in points and
  `viewports[ROOT].native_pixels_per_point = Some(dpr)`.
- A frame's `w`, `h` and its rectangles: **device pixels** (`round(w_css * dpr)`), at most 4096 per
  side (a larger pane is drawn at a lower dpr, which `frame.dpr` reports). The pane itself may be up
  to 16384 CSS px a side (the host refuses more); the UI is laid out over all of it.
- Times: milliseconds, as durations only. The two processes' clocks are never compared.

## Starting (leg A)

```
solidcraft-render-server --stream 127.0.0.1:PORT [--session ID] [--size WxH] [--dpr N] [--lang zh] [--theme dark|light] [--config-dir DIR]
```

`--config-dir DIR` (absolute; a relative one is refused: exit 1 before `hello`, the reason on stderr): SolidCraft's
per-user files are kept in `DIR` instead of the user's own folders (`%LOCALAPPDATA%\SolidCraft`,
`~/Library/Application Support/SolidCraft` and `~/Library/Caches/SolidCraft`, `~/.local/share/solidcraft`
and `~/.cache/solidcraft`), which a desktop install of SolidCraft shares: the recovery folder is
`DIR/recovery`, crash reports `DIR/crashes`, and the data and cache folders (the pictures of opened
designs in `DIR/thumbnails`, agent chat, plugins, files sent to a slicer) `DIR` itself
(`solidcraft_engine::recovery::set_user_dir`). `SOLIDCRAFT_RECOVERY_DIR` and `SOLIDCRAFT_CRASH_DIR`
still win. The graphics driver's own shader cache (Mesa's, Windows' `D3DSCache`) stays the driver's. The editing UI's preferences and
keyboard shortcuts are not written anywhere: the render server has no settings store (upstream's
desktop app keeps them in its window toolkit's storage), so a change lasts as long as the process.
Without the flag the user's own folders are used, as upstream's are.

Environment: `SOLIDCRAFT_RENDER_TOKEN` (64 lowercase hex, required: sent back in `hello`, proves the
connection is the process the host started) and `SOLIDCRAFT_CONTROL_TOKEN` (64 lowercase hex: the
control channel's token, the existing variable of docs/control-protocol.md; the host gives the same
token to the `solidcraft-cli mcp --connect` bridge). The render server writes **no** token file and
**no** `control/current.port` discovery file: it must never be found by a user's own
`solidcraft-cli mcp`, nor take the desktop app's place.

M1 mode, no host: `--snapshot DIR --frames N [--open FILE] [--size WxH] [--dpr N]` draws N frames,
writes `DIR/frame-0001.png` … and `DIR/frames.json` (each frame's `timing`, the adapter, the samples,
and `gpuMemory`: what the device holds by label where the backend can say, Vulkan and D3D12),
and exits 0. It also opens the control channel and prints `control 127.0.0.1:PORT` on stderr, so M1
and M2 scripts can ask `ui.handles`, `ui.worldToScreen`, `document.revision`.

The scene's MSAA is chosen once, at start, from `--dpr`: 1 sample at dpr ≥ 2, else 2 where the
device has them (else 1); a later `resize` to another dpr keeps it (`ready.samples` says what is in
effect). The frame is JPEG-encoded while it moves, and 4× targets were most of the device's memory.

Local measurement flags, never passed by the host (SPIKE.md): `--adapter auto|hardware|software`
and `--msaa auto|1|2|4` (either mode), and in M1 mode `--script FILE` (wire `InputEvent`s at given
frames), `--hold-ms N` (keep answering the control channel after the last frame), `--gap-ms N`
(between frames); with no `SOLIDCRAFT_CONTROL_TOKEN` M1 mode makes a token and prints it as
`control-token …`.

Sequence:

1. render server connects, opens the graphics device (`hello` names it), sends `hello` within 15 s
   of starting. When no device opens it sends nothing, writes the reason on stderr and exits 1:
   the host sees an exit before `ready` (its session goes to the viewer at once), not a crash.
2. host checks `v` and the token (constant time); wrong: `error` and close. Right: the host sends
   `policy`, `resize`, `visible` (current state; `resize` may equal the flags).
3. render server builds the pipelines, opens the control channel on `127.0.0.1:0`, draws the first
   frame, sends `ready` (within 60 s of `hello`).
4. steady state: `frame` / `cursor` / `ime` / `copied` / `ask_*` up; `input` / `resize` / `visible` /
   `ack` / `policy` / `answer` down.
5. end: host sends `stop` → render server finishes the frame, stops the control channel, sends `bye`,
   exits within 5 s (else the host kills it). The render server also exits, with `bye`
   `host-gone` if it can still write, when leg A closes or errors: it never outlives its host.
   An exit without `bye` is a crash to the host.

## Messages

Field names are exactly as written (JSON). Rust: one enum per direction with
`#[serde(tag = "k", rename_all = "snake_case")]`; struct fields `#[serde(rename_all = "camelCase")]`
(the multi-word fields are `inputSeq`, `controlPort`, `decodeMs`, `readOnly`). Optional fields
(`detail`) are omitted when absent; `null` is written where the type says nullable.

### Render server → host

| `k` | fields | when |
|---|---|---|
| `hello` | `v: u32` (=1), `token: string`, `pid: u32`, `version: string` (the app version), `backend: "metal"\|"dx12"\|"vulkan"\|"gl"\|null`, `adapter: string`, `software: bool` | first message |
| `ready` | `controlPort: u16`, `w: u32`, `h: u32` (device px), `dpr: f32`, `samples: u32` (scene MSAA in effect) | control channel up, first frame drawn |
| `frame` | `seq: u64` (from 1), `inputSeq: u64` (highest `input.seq` the pass read; 0 none), `w: u32`, `h: u32`, `dpr: f32`, `full: bool`, `rects: [FrameRect]`, `timing: FrameTiming`; payload | below |
| `cursor` | `icon: CursorIcon` | when the cursor changes |
| `ime` | `ime: ImeState \| null` | when it changes |
| `copied` | `text: string` | the UI copied text (already on the system clipboard) |
| `ask_save` | `ask: u32`, `name: string`, `exts: [string]` (no dot) | `Services::pick_save` was called |
| `ask_open` | `ask: u32`, `exts: [string]` (empty: any) | `Services::pick_open` / `pick_file` was called |
| `ask_done` | `ask: u32`, `ok: bool` | the pass after the answer's completion ran (the export command returned) |
| `bye` | `why: "stop"\|"host-gone"\|"device-lost"\|"error"`, `detail?: string` | last message |
| `error` | `code: "version"\|"token"\|"malformed"\|"too-large"\|"unknown-kind"\|"order"`, `message: string` | then close |

### Host → render server

| `k` | fields | effect |
|---|---|---|
| `input` | `seq: u64` (increasing from 1 per page attach; the render server only echoes the last one it read as `frame.inputSeq` and never checks the order), `events: [InputEvent]` | appended to the next pass's `RawInput.events`, in order |
| `resize` | `w: u32`, `h: u32` (CSS px), `dpr: f32` | new target size; the next frame is `full` |
| `visible` | `visible: bool` | false: passes still run (control requests, timers) but nothing is read back or encoded; true: the next frame is `full` |
| `ack` | `seq: u64`, `decodeMs: f32` | one credit back (start: 2 credits) |
| `policy` | `readOnly: string \| null` | read-only on with this reason, or off |
| `answer` | `ask: u32`, `path: string \| null` | resolves that ask's picker future; null = cancel |
| `stop` | `why: string` | finish and exit |
| `error` | as above | then close |

A save's `answer` is a place of the host's (its staging folder): the UI writes the file there, the
host delivers it after `ask_done` and tells the user where it went. The UI's own status message that
names that place is not shown (an error names the file alone), so the user reads only the host's
location; nothing else on the wire changes.

### Host ↔ page only (the render server never sees these)

`attach {v, session, w, h, dpr, theme, lang}` (page → host, first), `choose_cancel {ask}`,
`restart {}`; `status {state: "starting"|"live"|"fallback"|"stopped", reason?, readOnly}`,
`choose_file {ask, exts}`, `notice {level: "info"|"warn", text}` (host → page). Listed so the kinds
never collide.

## Shared types

**FrameRect** `{x: u32, y: u32, w: u32, h: u32, codec: "jpeg"|"png"|"rgba", ew: u32, eh: u32, offset: u32, length: u32}`:
a changed rectangle in device pixels; `ew`×`eh` is the encoded image's size (v1 always `= w`×`h`; a
later draft mode may send it smaller and the page scales it); `offset`/`length` locate its bytes in
the payload. `rgba`: `ew*eh*4` bytes, rows top to bottom, no padding, straight alpha = 255.
`jpeg`/`png`: a complete file each.

**FrameTiming** `{pass: f32, paint: f32, readback: f32, diff: f32, encode: f32}` ms: the UI pass
(`raw_input_hook` + `logic` + `ui`), tessellate + GPU encode/submit, copy back to the CPU (map
wait included), finding the changed tiles, encoding.

**Frame rules.** The page keeps the last frame and draws each rectangle over it. `full: true` means
the rectangles cover the whole `w`×`h`; it is sent first, after every `resize`, after `visible: true`,
and whenever `w`/`h`/`dpr` change. A frame is produced only when the pass painted something different
and a credit is left; while credits are 0 the changed region keeps growing (it is relative to the last
frame *sent*, so nothing is lost) and the next frame carries all of it. At most 60 frames a second.

**InputEvent**: an object with `"e"`. Rust: `#[serde(tag = "e", rename_all = "snake_case")]`.
Each maps to one `egui::Event` (egui 0.37, `crates/egui/src/data/input/event.rs`):

| `e` | fields | `egui::Event` |
|---|---|---|
| `move` | `x, y: f32` | `PointerMoved(pos2(x, y))` |
| `button` | `x, y: f32`, `button: "primary"\|"secondary"\|"middle"\|"extra1"\|"extra2"`, `pressed: bool`, `mods: Mods` | `PointerButton { pos, button, pressed, modifiers }` |
| `gone` | — | `PointerGone` |
| `wheel` | `dx, dy: f32` (already the UI's sign: −DOM delta), `unit: "point"\|"line"\|"page"`, `mods` | `MouseWheel { unit, delta: vec2(dx, dy), phase: TouchPhase::Move, source: MouseWheelSource::Unknown, modifiers }` |
| `zoom` | `factor: f32` | `Zoom(factor)` |
| `key` | `key: string` (DOM `key`), `physical: string \| null` (DOM `code`), `pressed, repeat: bool`, `mods` | `Key { key: Key::from_name(key).or(Key::from_name(physical)), physical_key: physical.and_then(Key::from_name), pressed, repeat, modifiers }`; dropped when neither name maps |
| `text` | `text` | `Text(text)` |
| `preedit` | `text`, `range: [u32, u32] \| null` (chars) | `Ime(ImeEvent::Preedit { text, active_range_chars: range.map(\|[a, b]\| a..b) })` |
| `commit` | `text` | `Ime(ImeEvent::Commit(text))` |
| `delete_surrounding` | `before, after: u32` | `Ime(ImeEvent::DeleteSurrounding { before_chars, after_chars })` |
| `copy` / `cut` | — | `Copy` / `Cut` |
| `paste` | `text` | `Paste(text)` |
| `focus` | `focused: bool` | `WindowFocused(focused)`, and `RawInput.focused` |
| `mods` | `mods` | `ModifiersChanged(modifiers)` |

**Mods** `{alt, ctrl, shift, meta: bool}` → `egui::Modifiers { alt, ctrl, shift, mac_cmd, command }`
with, on macOS, `mac_cmd = meta, command = meta`, elsewhere `mac_cmd = false, command = ctrl` (what
egui-winit does for a native window; the page cannot know which system the render server runs on).

**CursorIcon**: the `egui::CursorIcon` variant name (`Default`, `None`, `ContextMenu`, `Help`,
`PointingHand`, `Progress`, `Wait`, `Cell`, `Crosshair`, `Text`, `VerticalText`, `Alias`, `Copy`,
`Move`, `NoDrop`, `NotAllowed`, `Grab`, `Grabbing`, `AllScroll`, `ResizeHorizontal`, `ResizeNeSw`,
`ResizeNwSe`, `ResizeVertical`, `ResizeEast`, `ResizeSouthEast`, `ResizeSouth`, `ResizeSouthWest`,
`ResizeWest`, `ResizeNorthWest`, `ResizeNorth`, `ResizeNorthEast`, `ResizeColumn`, `ResizeRow`,
`ZoomIn`, `ZoomOut`), from `PlatformOutput.cursor_icon`.

**ImeState** `{purpose: "normal"|"password"|"terminal", rect: [x, y, w, h], cursor: [x, y, w, h], interrupt: bool}`
from `PlatformOutput.ime` (`IMEOutput { purpose, rect, cursor_rect, should_interrupt_composition }`),
in points. The page puts its hidden text area at `cursor`, so the system's candidate box opens at the
caret.

## Product name

The UI says SolidZ wherever upstream's says SolidCraft (every catalog message, in every language, the
engine's messages on the status line and in dialogs, copied text, the exported shortcuts file's name),
and offers none of SolidCraft's own ways out: its documentation and release pages, Report an Issue…
(in Help and in command search), Create Issue…, crash reports, its update check, its command-line
install, its community links. `diagnostics.report_issue` refuses in this process whoever asks (the
UI or a client of the control channel): "当前不可用：这里不能发送问题报告"; nothing is gathered or sent.
Upstream's Assistant (a chat that starts coding agents installed on the computer) is not offered:
`ui.assistant` and `ui.agent_chat` refuse ("这里不提供助手，请用对话里的助手"), and no panel, key, search
row or agent process exists; the host's own chat is the assistant.
Fixed in the render server; there is no flag for it.

## What the render server does with the platform output

- `cursor_icon` → `cursor` (on change). `ime` → `ime` (on change).
- `OutputCommand::CopyText(t)` → the system clipboard (arboard, already a dependency of the UI crate),
  then `copied`. `CopyImage` → the system clipboard only. `OpenUrl` and `TextSelectionSettled` →
  ignored (logged).
- Viewport commands: `Close`, `Minimized`, `Maximized`, `BeginResize`, `StartDrag`, `Title`, `Focus`,
  `CursorPosition` are ignored; the pane is not a window.

## Read-only (`policy`)

`readOnly: "<reason>"` turns the engine's host read-only mode on with that reason (a command that
changes the design is undone and refused with it; `file.save`, `file.save_as`, undo/redo refused;
opening, looking, selecting, measuring and exporting stay). The render server also keeps the
`document.revision` it had when the policy arrived and, if a pass ends with another one while
read-only, puts the design back with `document.import` of the text it exported then, and shows the
reason in the status line. `null` turns both off.
