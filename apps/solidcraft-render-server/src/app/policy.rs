//! Host read-only (PROTOCOL.md, "Read-only"): the engine's own hook refuses and undoes every
//! change (`solidcraft_engine::read_only`, E2), and a backstop here: the design as it was when
//! read-only came on is kept, and a pass that ends with the same design at another revision puts
//! it back (`document.import`) and says why on the status line. Another design opened meanwhile
//! (`doc.open` stays allowed: reading) is kept in its turn, not undone.

use serde_json::json;
use solidcraft_ui_egui::SolveApp;

/// The design kept while read-only: which one, at which revision, and its text.
struct Kept {
    document_id: String,
    revision: u64,
    text: String,
}

#[derive(Default)]
pub struct ReadOnly {
    reason: Option<String>,
    kept: Option<Kept>,
}

impl ReadOnly {
    /// `policy {readOnly}`: on with this reason, or off.
    pub fn set(&mut self, app: &mut SolveApp, reason: Option<String>) {
        let reason = reason.map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
        solidcraft_engine::read_only::set(reason.clone());
        self.kept = None;
        if let Some(why) = &reason {
            self.kept = keep(app);
            app.set_status(why.clone(), false);
        }
        self.reason = reason;
    }

    /// After every pass while read-only: a change the hook did not catch is undone.
    pub fn after_pass(&mut self, app: &mut SolveApp) {
        let (Some(why), Some(kept)) = (&self.reason, &self.kept) else { return };
        let revision = app.session.design_revision();
        if app.session.document_id != kept.document_id {
            // Another design was opened: that one is now the one kept.
            self.kept = keep(app);
            return;
        }
        if revision == kept.revision {
            return;
        }
        // `crates/ui-egui/src/documents.rs:734`: opens the text saved, in the active design's place.
        match solidcraft_ui_egui::documents::import(app, &json!({"text": kept.text})) {
            Ok(_) => {
                log::warn!("read-only: the design changed (revision {} -> {revision}); put back", kept.revision);
                app.set_status(why.clone(), true);
                self.kept = keep(app);
            }
            // A file operation in progress: tried again after the next pass.
            Err(e) => log::warn!("read-only: could not put the design back yet: {e}"),
        }
    }
}

/// The active design as `document.export` gives it (`crates/ui-egui/src/documents.rs:723`).
fn keep(app: &mut SolveApp) -> Option<Kept> {
    match solidcraft_ui_egui::documents::export(app) {
        Ok(v) => Some(Kept {
            document_id: app.session.document_id.clone(),
            revision: app.session.design_revision(),
            text: v.get("text").and_then(serde_json::Value::as_str).unwrap_or_default().to_string(),
        }),
        Err(e) => {
            log::warn!("read-only: the design could not be kept: {e}");
            None
        }
    }
}
