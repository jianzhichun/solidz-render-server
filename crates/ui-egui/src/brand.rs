//! A build that carries SolidCraft inside another product: the host names it once at start
//! (`solidcraft_ui_text::set_product_name`), the messages say that name, and SolidCraft's own ways
//! out are not offered there — its issue tracker (Report an Issue…, Create Issue…, a crash's
//! report), its documentation and release pages, its community links, its update check and its
//! command-line install. They are the product's to offer. Nothing changes while no host names one.

use std::cell::RefCell;

thread_local! {
    /// Commands the host does not offer in this UI, and why ([`refuse_commands`]). Per thread:
    /// the UI and its commands run on one.
    static REFUSED: RefCell<(Vec<String>, String)> = const { RefCell::new((Vec::new(), String::new())) };
}

/// Refuse the commands `ids` in the UI on this thread, with `reason` (the host's words): they
/// are refused when run or started, have no key and are not offered by command search; for
/// `ui.assistant`, the Assistant is not shown and starts no agent. An empty list refuses nothing.
pub fn refuse_commands(ids: &[&str], reason: &str) {
    REFUSED.set((ids.iter().map(|id| id.to_string()).collect(), reason.to_string()));
}

/// Why the host refuses `id` here; `None` when it does not.
pub fn refused(id: &str) -> Option<String> {
    REFUSED.with(|r| {
        let r = r.borrow();
        r.0.iter().any(|refused| refused == id).then(|| r.1.clone())
    })
}

/// The host's refusal of `id` as a command's error ("`id` is not available right now: why"),
/// which the interface language reads as "<command>当前不可用：why".
pub fn refusal(id: &str) -> Option<String> {
    refused(id).map(|why| solidcraft_engine::EngineError::Disabled(id.to_string(), why).to_string())
}

/// A host carries SolidCraft under another product's name.
pub fn hosted() -> bool {
    solidcraft_ui_text::product_name() != solidcraft_ui_text::PRODUCT_NAME
}

/// SolidCraft's own issue tracker (a GitHub draft or its ticket server) is offered.
pub fn own_report_flow() -> bool {
    !hosted()
}

/// A Help item that leads to SolidCraft itself (its pages, tracker, updates, command-line tools)
/// rather than to this window's own content. `id`: a Help item or a link's id.
pub fn leads_to_solidcraft(id: &str) -> bool {
    matches!(id, "docs" | "version" | "updates" | "report" | "crashes" | "connect" | "install_cli") || crate::links::by_id(id).is_some()
}

/// A command for SolidCraft's own issue tracker or crash reports (Report an Issue, its status,
/// Crash Reports): command search does not offer it under a host's name.
pub fn hides_command(id: &str) -> bool {
    refused(id).is_some() || (hosted() && (id.starts_with("diagnostics.report_issue") || id.starts_with("diagnostics.crash_report")))
}

#[cfg(test)]
mod tests {
    /// Unnamed, the build is SolidCraft's own and offers everything (the name is process-wide, so
    /// the hosted case is tested where the host names it: the render server).
    #[test]
    fn unnamed_offers_solidcraft_s_own_ways_out() {
        assert!(!super::hosted());
        assert!(super::own_report_flow());
        assert!(super::leads_to_solidcraft("report") && super::leads_to_solidcraft("kofi") && !super::leads_to_solidcraft("about"));
        assert!(!super::hides_command("diagnostics.report_issue"));
    }

    /// A command the host refuses is refused whichever way it comes (run, started, its key, the
    /// Assistant's own control) and command search does not offer it; offered again once not refused.
    #[test]
    fn a_refused_assistant_is_refused_and_not_offered() {
        let mut h = crate::scenario::Harness::new();
        let offered = |app: &crate::SolveApp| crate::shortcut_box::search(app, "assistant").iter().any(|row| row.0 == "ui.assistant");
        assert!(offered(&h.app) && crate::agent_chat::shown(&h.app));
        super::refuse_commands(&["ui.assistant"], "这里不提供助手");
        assert!(!offered(&h.app) && !crate::agent_chat::shown(&h.app));
        assert!(crate::keymap::keys(&h.app, "ui.assistant").is_empty(), "no key runs it");
        let run = h.app.run("ui.assistant", serde_json::json!({})).unwrap_err();
        assert!(run.contains("这里不提供助手"), "{run}");
        h.app.start("ui.assistant");
        assert!(!h.app.agent_chat.open);
        let ctx = h.ctx.clone();
        let control = crate::agent_chat::control(&mut h.app, &ctx, &serde_json::json!({"connect": true})).unwrap_err();
        assert!(control.contains("这里不提供助手"), "{control}");
        super::refuse_commands(&[], "");
        assert!(offered(&h.app));
    }
}
