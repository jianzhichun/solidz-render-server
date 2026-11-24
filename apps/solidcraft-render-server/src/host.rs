//! The host's editor, not SolidCraft's app: the product's name in every message, SolidCraft's own
//! ways out refused (its problem reports, its Assistant), and the UI in `embed_editor` mode (no
//! application bar, start page or file commands of its own). The binary applies it; the brand
//! scan (`tests/brand_scan.rs`) applies the same and walks what a user can reach.

use solidcraft_ui_egui::SolveApp;

/// The product this editor is part of: the UI's messages name it where they name SolidCraft, and
/// SolidCraft's own report, update and community entries are left out
/// (`solidcraft_ui_egui::brand`).
pub const PRODUCT: &str = "SolidZ";

/// Why `diagnostics.report_issue` refuses here (shown as "<command>当前不可用：<this>").
pub const REPORTS_REFUSED: &str = "这里不能发送问题报告";

/// Why upstream's Assistant is refused here (shown as "<command>当前不可用：<this>"): the host's chat
/// is the assistant, and an agent started from this UI would write and export around the host
/// (no `ask_save`, so no trial mark or read-only check).
pub const ASSISTANT_REFUSED: &str = "这里不提供助手，请用对话里的助手";

/// Process-wide, before any message is read: the product's name, and SolidCraft's own problem
/// reports (system info, the log, a screenshot to its ticket site) never leave from here, whoever
/// asks: the user's UI or the model over the control channel.
pub fn name_product() {
    solidcraft_ui_text::set_product_name(PRODUCT);
    solidcraft_engine::io::set_mesh_producer(PRODUCT);
    solidcraft_engine::report_issue::refuse(Some(REPORTS_REFUSED));
}

/// Why EasyPrint is refused here: the file would go from this editor to an outside website, around
/// the host's exports (no `ask_save`, so no trial mark or read-only check).
pub const OUTSIDE_SERVICE_REFUSED: &str = "这里不发送到外部服务，请导出后自己上传";
/// 0.46's app bar (Assistant, Help, Report a bug, Donate, Discord tiles) is not drawn in an embedded editor; its customise and
/// reset commands would open that bar's tile picker all the same, so the host refuses them.
pub const APP_BAR_REFUSED: &str = "这个编辑界面没有应用栏";

/// On the UI's thread, before its app is made: no Assistant panel, key, search row or agent
/// process, and no upload to an outside service (EasyPrint).
pub fn refuse_on_this_thread() {
    solidcraft_ui_egui::brand::refuse_commands(&[
        ("ui.assistant", ASSISTANT_REFUSED),
        ("file.open_in_easyprint", OUTSIDE_SERVICE_REFUSED),
        ("ui.appBarCustomize", APP_BAR_REFUSED),
        ("ui.appBarReset", APP_BAR_REFUSED),
    ]);
    // the plugin reads the design's rebuild script after the assistant's calls; a design it cannot be written for (bodies moved
    // between components) refuses it, and the plugin reads that from the answer: not a message for the user
    solidcraft_ui_egui::brand::quiet_commands(&["timeline.export_script"]);
}

/// The app as the host's editor (#8517: no app bar, start page, New/Open/Save/Close; ribbon,
/// browser, timeline and properties stay).
pub fn editor(app: &mut SolveApp) {
    app.embed_editor = true;
}
