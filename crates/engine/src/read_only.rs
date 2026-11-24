//! Host read-only mode: an embedding host (SolidZ, when its licence has expired, or while
//! another session holds the design) lets the design be opened, looked at and exported, and
//! refuses every change to it with a reason the user and the model are shown.
//!
//! Held per process, not per `Session` or per thread: new/open/recover replace the `Session`,
//! and the UI applies slow edits on worker threads (`apply_off_thread`, the control channel's
//! off-thread jobs), which must be refused just like the window's own commands.
//!
//! The rule is on the outcome, not on a list of commands: a command that left the design (or
//! what is hidden) different is undone and refused (`Session::execute`'s error path puts it
//! back), so a command added later is covered. Only what writes the design's file without
//! changing it (`file.save`, `file.save_as`) and what puts back another state (undo, redo,
//! recover) is named. Opening a design (`doc.open`), looking, selecting, measuring and
//! exporting stay.

use std::sync::{Mutex, PoisonError};

static REASON: Mutex<Option<String>> = Mutex::new(None);

/// Commands that do not change the design in memory but write it, or replay history over it.
const REFUSED: &[&str] = &["file.save", "file.save_as", "edit.undo", "edit.redo", "edit.repeat", "doc.recover"];

/// Commands that replace the design by another one the user chose: reading, not changing.
const OPENS: &[&str] = &["doc.open"];

/// Longest reason kept (characters): it goes into every refusal.
const MAX_REASON_CHARS: usize = 200;

/// Turn read-only on with the reason shown to the user and the model, or off (`None`, or
/// nothing but spaces).
pub fn set(reason: Option<String>) {
    *REASON.lock().unwrap_or_else(PoisonError::into_inner) = normalized(reason);
}

/// The reason, while read-only is on.
pub fn reason() -> Option<String> {
    REASON.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Why `command` may not run at all now, before it does.
pub fn refusal(command: &str) -> Option<String> {
    refusal_under(reason(), command)
}

/// Why a command that ran and changed the design must be undone, after it ran.
pub fn change_refusal(command: &str) -> Option<String> {
    change_refusal_under(reason(), command)
}

fn normalized(reason: Option<String>) -> Option<String> {
    reason.map(|r| r.trim().chars().take(MAX_REASON_CHARS).collect::<String>()).filter(|r| !r.is_empty())
}

fn refusal_under(reason: Option<String>, command: &str) -> Option<String> {
    reason.filter(|_| REFUSED.contains(&command))
}

fn change_refusal_under(reason: Option<String>, command: &str) -> Option<String> {
    reason.filter(|_| !OPENS.contains(&command))
}

// The process-wide switch itself is tested in its own process (tests/read_only.rs): turning it
// on here would refuse the commands of every other test running beside these.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_reasons_turn_it_off_and_long_ones_are_cut() {
        assert_eq!(normalized(Some("  ".into())), None);
        assert_eq!(normalized(Some(" 授权已到期 ".into())).as_deref(), Some("授权已到期"));
        assert_eq!(normalized(Some("字".repeat(500))).map(|r| r.chars().count()), Some(MAX_REASON_CHARS));
    }

    #[test]
    fn on_refuses_saving_and_changes_but_not_opening() {
        let on = || Some("授权已到期".to_string());
        assert_eq!(refusal_under(on(), "file.save").as_deref(), Some("授权已到期"));
        assert!(refusal_under(on(), "view.fit").is_none());
        assert!(change_refusal_under(on(), "solid.box").is_some());
        assert!(change_refusal_under(on(), "doc.open").is_none());
        assert!(refusal_under(None, "file.save").is_none() && change_refusal_under(None, "solid.box").is_none());
    }
}
