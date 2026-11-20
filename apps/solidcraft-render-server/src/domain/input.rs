//! The page's input, already in the UI's vocabulary (PROTOCOL.md, "InputEvent"), mapped 1:1 to
//! `egui::Event` (egui 0.37: `crates/egui-0.37.0/src/data/input/event.rs:18-206`). The browser-side
//! translation (DOM events to these) lives in the pane, ported from eframe's web runner.

use serde::Deserialize;
use serde_json::Value;

/// The keys held, as the page sees them.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct ModKeys {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub meta: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ButtonName {
    Primary,
    Secondary,
    Middle,
    Extra1,
    Extra2,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WheelUnit {
    Point,
    Line,
    Page,
}

/// One input event as the wire carries it (`"e"` names the kind).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "e", rename_all = "snake_case")]
pub enum InputEvent {
    Move { x: f32, y: f32 },
    Button { x: f32, y: f32, button: ButtonName, pressed: bool, mods: ModKeys },
    Gone,
    Wheel { dx: f32, dy: f32, unit: WheelUnit, mods: ModKeys },
    Zoom { factor: f32 },
    Key { key: String, physical: Option<String>, pressed: bool, repeat: bool, mods: ModKeys },
    Text { text: String },
    Preedit { text: String, range: Option<[u32; 2]> },
    Commit { text: String },
    DeleteSurrounding { before: u32, after: u32 },
    Copy,
    Cut,
    Paste { text: String },
    Focus { focused: bool },
    Mods { mods: ModKeys },
}

/// Read each event on its own; one this version does not know (or that does not read) is
/// dropped and counted, the rest go on.
pub fn parse_events(values: &[Value]) -> (Vec<InputEvent>, usize) {
    let mut dropped = 0;
    let events = values
        .iter()
        .filter_map(|v| {
            let parsed = InputEvent::deserialize(v).ok();
            dropped += usize::from(parsed.is_none());
            parsed
        })
        .collect();
    (events, dropped)
}

/// How page input becomes UI input on this system.
#[derive(Clone, Copy, Debug)]
pub struct Translator {
    /// The render server runs on macOS: the command key is meta there, ctrl elsewhere (what
    /// egui-winit does for a native window; the page cannot know).
    pub mac: bool,
}

impl Translator {
    pub fn native() -> Self {
        Translator { mac: cfg!(target_os = "macos") }
    }

    /// `egui::Modifiers` (`data/input/modifiers.rs:19-37`) for the keys the page holds.
    pub fn modifiers(&self, m: ModKeys) -> egui::Modifiers {
        egui::Modifiers { alt: m.alt, ctrl: m.ctrl, shift: m.shift, mac_cmd: self.mac && m.meta, command: if self.mac { m.meta } else { m.ctrl } }
    }

    /// The UI event for `ev`. `points_per_css`: UI points per CSS pixel (1 / the UI's zoom),
    /// for positions and point-unit wheel deltas. `None`: an event the UI has no use for (a key
    /// neither of whose names it knows, empty text).
    pub fn event(&self, ev: InputEvent, points_per_css: f32) -> Option<egui::Event> {
        let at = |x: f32, y: f32| egui::pos2(x * points_per_css, y * points_per_css);
        Some(match ev {
            InputEvent::Move { x, y } => egui::Event::PointerMoved(at(x, y)),
            InputEvent::Button { x, y, button, pressed, mods } => {
                egui::Event::PointerButton { pos: at(x, y), button: pointer_button(button), pressed, modifiers: self.modifiers(mods) }
            }
            InputEvent::Gone => egui::Event::PointerGone,
            InputEvent::Wheel { dx, dy, unit, mods } => {
                let scale = if unit == WheelUnit::Point { points_per_css } else { 1.0 };
                egui::Event::MouseWheel {
                    unit: wheel_unit(unit),
                    delta: egui::vec2(dx * scale, dy * scale),
                    phase: egui::TouchPhase::Move,
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: self.modifiers(mods),
                }
            }
            InputEvent::Zoom { factor } => egui::Event::Zoom(factor),
            InputEvent::Key { key, physical, pressed, repeat, mods } => {
                // Browser names (`ArrowDown`, `KeyA`, `Digit0`) are what `Key::from_name` reads
                // (`data/key.rs:503`), as eframe's web runner does it.
                let physical_key = physical.as_deref().and_then(egui::Key::from_name);
                let key = egui::Key::from_name(&key).or(physical_key)?;
                egui::Event::Key { key, physical_key, pressed, repeat, modifiers: self.modifiers(mods) }
            }
            InputEvent::Text { text } if text.is_empty() => return None,
            InputEvent::Text { text } => egui::Event::Text(text),
            InputEvent::Preedit { text, range } => {
                egui::Event::Ime(egui::ImeEvent::Preedit { text, active_range_chars: range.map(|[a, b]| a.min(b) as usize..a.max(b) as usize) })
            }
            InputEvent::Commit { text } => egui::Event::Ime(egui::ImeEvent::Commit(text)),
            InputEvent::DeleteSurrounding { before, after } => {
                egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: before as usize, after_chars: after as usize })
            }
            InputEvent::Copy => egui::Event::Copy,
            InputEvent::Cut => egui::Event::Cut,
            // As egui-winit pastes: Windows line ends become `\n`.
            InputEvent::Paste { text } => egui::Event::Paste(text.replace("\r\n", "\n")),
            InputEvent::Focus { focused } => egui::Event::WindowFocused(focused),
            InputEvent::Mods { mods } => egui::Event::ModifiersChanged(self.modifiers(mods)),
        })
    }
}

fn pointer_button(b: ButtonName) -> egui::PointerButton {
    match b {
        ButtonName::Primary => egui::PointerButton::Primary,
        ButtonName::Secondary => egui::PointerButton::Secondary,
        ButtonName::Middle => egui::PointerButton::Middle,
        ButtonName::Extra1 => egui::PointerButton::Extra1,
        ButtonName::Extra2 => egui::PointerButton::Extra2,
    }
}

fn wheel_unit(u: WheelUnit) -> egui::MouseWheelUnit {
    match u {
        WheelUnit::Point => egui::MouseWheelUnit::Point,
        WheelUnit::Line => egui::MouseWheelUnit::Line,
        WheelUnit::Page => egui::MouseWheelUnit::Page,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn one(t: Translator, v: Value, points_per_css: f32) -> Option<egui::Event> {
        let (mut events, dropped) = parse_events(&[v]);
        assert_eq!(dropped, 0);
        t.event(events.remove(0), points_per_css)
    }

    const PC: Translator = Translator { mac: false };
    const MAC: Translator = Translator { mac: true };
    const NONE: Value = Value::Null;

    fn mods(ctrl: bool, meta: bool) -> Value {
        json!({"alt": false, "ctrl": ctrl, "shift": false, "meta": meta})
    }

    #[test]
    fn pointer_events_land_in_points() {
        assert_eq!(one(PC, json!({"e": "move", "x": 10, "y": 4}), 0.5), Some(egui::Event::PointerMoved(egui::pos2(5.0, 2.0))));
        let click = one(PC, json!({"e": "button", "x": 1, "y": 2, "button": "secondary", "pressed": true, "mods": mods(false, false)}), 1.0);
        assert!(matches!(click, Some(egui::Event::PointerButton { button: egui::PointerButton::Secondary, pressed: true, .. })));
        let wheel = one(PC, json!({"e": "wheel", "dx": 0, "dy": -3, "unit": "line", "mods": mods(false, false)}), 0.5);
        assert!(matches!(wheel, Some(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta, .. }) if delta == egui::vec2(0.0, -3.0)));
    }

    #[test]
    fn the_command_key_follows_the_render_servers_system() {
        let key = json!({"e": "key", "key": "c", "physical": "KeyC", "pressed": true, "repeat": false, "mods": mods(true, false)});
        let Some(egui::Event::Key { modifiers, .. }) = one(PC, key.clone(), 1.0) else { panic!("a key") };
        assert!(modifiers.command && !modifiers.mac_cmd);
        let Some(egui::Event::Key { modifiers, .. }) = one(MAC, key, 1.0) else { panic!("a key") };
        assert!(!modifiers.command && modifiers.ctrl);
        let meta = json!({"e": "mods", "mods": mods(false, true)});
        assert!(matches!(one(MAC, meta, 1.0), Some(egui::Event::ModifiersChanged(m)) if m.command && m.mac_cmd));
    }

    #[test]
    fn keys_map_by_browser_names_and_unknown_ones_drop() {
        let key = |k: &str, p: Value| {
            one(PC, json!({"e": "key", "key": k, "physical": p, "pressed": true, "repeat": false, "mods": mods(false, false)}), 1.0)
        };
        assert!(matches!(key("ArrowDown", NONE), Some(egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, .. })));
        // A layout's own character falls back to the physical key.
        assert!(matches!(key("ф", json!("KeyA")), Some(egui::Event::Key { key: egui::Key::A, physical_key: Some(egui::Key::A), .. })));
        assert_eq!(key("Unidentified", NONE), None);
    }

    #[test]
    fn ime_events_carry_text_and_char_ranges() {
        let pre = one(PC, json!({"e": "preedit", "text": "测试", "range": [0, 2]}), 1.0);
        assert_eq!(pre, Some(egui::Event::Ime(egui::ImeEvent::Preedit { text: "测试".into(), active_range_chars: Some(0..2) })));
        let open = one(PC, json!({"e": "preedit", "text": "ce", "range": null}), 1.0);
        assert_eq!(open, Some(egui::Event::Ime(egui::ImeEvent::Preedit { text: "ce".into(), active_range_chars: None })));
        assert_eq!(one(PC, json!({"e": "commit", "text": "测试"}), 1.0), Some(egui::Event::Ime(egui::ImeEvent::Commit("测试".into()))));
        let shrink = one(PC, json!({"e": "delete_surrounding", "before": 1, "after": 0}), 1.0);
        assert_eq!(shrink, Some(egui::Event::Ime(egui::ImeEvent::DeleteSurrounding { before_chars: 1, after_chars: 0 })));
    }

    #[test]
    fn clipboard_focus_and_unknown_events() {
        assert_eq!(one(PC, json!({"e": "paste", "text": "a\r\nb"}), 1.0), Some(egui::Event::Paste("a\nb".into())));
        assert_eq!(one(PC, json!({"e": "copy"}), 1.0), Some(egui::Event::Copy));
        assert_eq!(one(PC, json!({"e": "focus", "focused": false}), 1.0), Some(egui::Event::WindowFocused(false)));
        let (events, dropped) = parse_events(&[json!({"e": "touch"}), json!({"e": "gone"}), json!({"e": "move"})]);
        assert_eq!((events, dropped), (vec![InputEvent::Gone], 2));
    }
}
