#![allow(clippy::print_stderr)] // The interactive scan's progress and findings are its console output.

//! The brand scan: what a user can reach in the host's editor (the render server's own setup,
//! `solidcraft_render_server::host`: SolidZ named, SolidCraft's reports and Assistant refused,
//! `embed_editor`) must not show SolidCraft's name, a link out of the product, or one of
//! SolidCraft's own ways out (its tracker, crash reports, updates, documentation, community,
//! Assistant). Not a list of the entries known today: it walks the UI and reads what is shown.
//!
//! Walked, in Chinese and in English, on upstream's headless app (`scenario::Harness`) with a body
//! in the design: every ribbon tab, every button's tooltip and every panel's drop-down; every
//! clickable widget (accessibility tree) and, one level down, every widget its click showed
//! (menus, dialogs and their buttons); right-click menus on the model, empty space, the browser's
//! rows and the ribbon; command search for a list of queries, as typed and as data (every command
//! it offers); every `ui.*` command started; an error and a plain toast. Read: every text painted
//! (galleys, so tooltips and toasts too), every accessible label and value, every URL a click
//! asked to open, every published handle name, and every key bound to a refused command.
//!
//! Run on every upstream tag: `cargo test -p solidcraft-render-server --test brand_scan -- --nocapture`.
//! A finding names the text and where it was reached; fix it in `crates/ui-egui/src/brand.rs`'s
//! hooks, or, after review, add it to [`ALLOW`] with the reason. `BRAND_SCAN_OUT=DIR` writes every
//! text shown and where (`brand-scan-<lang>.txt`) for that review.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use egui::accesskit::Action;
use serde_json::json;
use solidcraft_render_server::host;
use solidcraft_ui_egui::scenario::{self, Harness};

/// Shown text that names SolidCraft or leads out of the product (lowercase substrings).
const BRAND: &[&str] =
    &["solidcraft", "solid craft", "solvecraft", "tangency", "github", "discord", "ko-fi", "kofi", "http://", "https://", "www.", "mailto:"];

/// Shown text that offers one of SolidCraft's own ways out, or a service outside the product
/// (lowercase substrings).
const ENTRIES: &[&str] = &[
    "report an issue",
    "report a bug",
    "report issue",
    "create issue",
    "report…",
    "crash report",
    "check for updates",
    "update available",
    "update now",
    "release notes",
    "documentation",
    "assistant",
    "feedback",
    "donate",
    "sponsor",
    "community",
    "easyprint",
    "报告问题",
    "问题报告",
    "提交问题",
    "创建问题",
    "报告…",
    "崩溃报告",
    "检查更新",
    "可用更新",
    "立即更新",
    "发行说明",
    "版本说明",
    "帮助文档",
    "在线文档",
    "助手",
    "反馈",
    "捐赠",
    "赞助",
    "社区",
];

/// Shown labels that are such an entry only as a whole (lowercase, a trailing "…" dropped): words
/// too common inside other text ("文档" is also "document").
const EXACT: &[&str] = &["documentation", "docs", "help", "about", "文档", "帮助", "关于"];

/// A command or handle whose id names such an entry (lowercase substrings of the id).
const ENTRY_IDS: &[&str] = &[
    "report",
    "issue",
    "crash",
    "update",
    "assistant",
    "agent",
    "feedback",
    "help",
    "about",
    "community",
    "ticket",
    "donate",
    "discord",
    "kofi",
    "easyprint",
];

/// Reviewed exceptions: a shown text, a URL or an id, exactly; where it may show (a part of the
/// place the walk reached it, "" anywhere); and why it may stay. An id is also allowed by its last
/// `:` part (a command id under any handle prefix).
const ALLOW: &[(&str, &str, &str)] = &[
    // The Display menu's heading over Show agent cursor: the cursor of whatever assistant drives
    // this editor over the control channel, here the host's chat. Upstream's own Assistant panel
    // row under it is gone (agent_chat::shown, the host refuses ui.assistant).
    ("助手", "Button:显示", "Display menu heading over the agent cursor"),
    ("ASSISTANT", "Button:Display", "Display menu heading over the agent cursor"),
    ("view:agent_cursor", "", "Show agent cursor: the host's assistant acting in this editor"),
    // Commands that update a design's own parts.
    ("component.update_links", "", "Update Links: linked components from their files"),
    ("sketch.update_references", "", "Refresh sketch references after their source changes; not an application update"),
    ("note.update", "", "a note's text"),
    ("config.update_from_design", "", "Update From Design: a configuration row's values from the design (0.46)"),
];

/// Handle prefixes that are a kind of widget, not an entry: `ui_kit::Text::Report` is a dialog's
/// report-style line (a result or a status), not a problem report.
const ALLOW_ID_PREFIXES: &[(&str, &str)] = &[("dialog:report:", "a dialog's report-style text line")];

/// Command search queries typed into the ribbon's search box.
const QUERIES: &[&str] = &[
    "solidcraft",
    "help",
    "about",
    "report",
    "issue",
    "bug",
    "crash",
    "update",
    "assistant",
    "agent",
    "ai",
    "feedback",
    "docs",
    "discord",
    "github",
    "community",
    "settings",
    "帮助",
    "关于",
    "报告",
    "问题",
    "更新",
    "助手",
    "反馈",
    "文档",
    "设置",
];

/// Clicks after which the walk stops (a guard against a UI that keeps growing).
const MAX_CLICKS: usize = 4000;

/// What a slicer would receive, before any program is started. The model owns its names and face tags.
#[test]
fn mesh_writers_name_the_host_but_leave_the_model_alone() {
    use solidcraft_engine::{geom::Mesh, io};
    host::name_product();
    let name = "SolidCraft coupon";
    let mut mesh = Mesh { positions: vec![Default::default(); 3], triangles: vec![[0, 1, 2]], ..Default::default() };
    mesh.positions[1].x = 1.0;
    mesh.positions[2].y = 1.0;
    let stl = io::stl_binary(std::slice::from_ref(&mesh), name);
    assert_eq!(std::str::from_utf8(&stl[..80]).unwrap().trim_end(), "SolidZ STL SolidCraft coupon");
    assert_eq!((stl.len(), u32::from_le_bytes(stl[80..84].try_into().unwrap())), (84 + 50, 1), "one triangle, each 50 bytes");
    let object = io::MeshObject {
        name: name.into(),
        positions: mesh.positions,
        triangles: mesh.triangles,
        tri_faces: vec![0],
        faces: 1,
        ..Default::default()
    };
    let xml = io::model_xml_with(&[object], &[("Title", name)]);
    assert!(xml.contains("<metadata name=\"Application\">SolidZ</metadata>"));
    assert!(xml.contains("<metadata name=\"Title\">SolidCraft coupon</metadata>"));
    assert!(xml.contains("name=\"SolidCraft coupon\""));
    assert!(xml.contains("ft:face=\"0\""));
}

#[test]
fn nothing_shown_leads_to_solidcraft() {
    host::name_product();
    host::refuse_on_this_thread();
    let mut findings = Vec::new();
    for lang in ["zh", "en"] {
        let started = std::time::Instant::now();
        let mut scan = Scan::new(lang);
        scan.walk();
        let found = scan.findings();
        eprintln!(
            "brand scan {lang}: {} texts, {} ids, {} URLs, {} clicks, {} fresh apps, {:.0} s, {} findings",
            scan.seen.len(),
            scan.ids.len(),
            scan.opened.len(),
            scan.clicks,
            scan.fresh,
            started.elapsed().as_secs_f64(),
            found.len()
        );
        scan.write_report(&found);
        findings.extend(found.into_iter().map(|f| format!("[{lang}] {f}")));
    }
    assert!(findings.is_empty(), "{} findings:\n{}", findings.len(), findings.join("\n"));
}

/// One language's walk and what it saw: text, ids and URLs, each with the first place reached.
struct Scan {
    lang: &'static str,
    h: Harness,
    seen: BTreeMap<String, String>,
    ids: BTreeMap<String, String>,
    opened: BTreeMap<String, String>,
    keyed: Vec<String>,
    clicked: BTreeSet<String>,
    clicks: usize,
    fresh: usize,
}

impl Scan {
    fn new(lang: &'static str) -> Self {
        Scan {
            lang,
            h: fresh(lang),
            seen: BTreeMap::new(),
            ids: BTreeMap::new(),
            opened: BTreeMap::new(),
            keyed: Vec::new(),
            clicked: BTreeSet::new(),
            clicks: 0,
            fresh: 1,
        }
    }

    fn walk(&mut self) {
        let started = std::time::Instant::now();
        let progress = |scan: &Scan, what: &str| {
            eprintln!("  {} {what}: {} clicks, {} fresh apps, {:.0} s", scan.lang, scan.clicks, scan.fresh, started.elapsed().as_secs_f64());
        };
        self.look("start");
        self.offered();
        let tabs: Vec<String> = scenario::handles("tab:").into_iter().map(|(name, _)| name).collect();
        for tab in &tabs {
            self.tab(tab);
            progress(self, tab);
        }
        self.context_menus();
        progress(self, "context menus");
        self.search();
        self.toasts();
        self.ui_commands();
        progress(self, "search, toasts, ui commands");
    }

    /// Read what the last frame showed, and every URL opened since.
    fn look(&mut self, place: &str) {
        for text in shown(&self.h) {
            self.seen.entry(text).or_insert_with(|| place.to_string());
        }
        self.look_ids(place);
    }

    /// The handles the last frame published, and every URL opened since.
    fn look_ids(&mut self, place: &str) {
        for (name, _) in scenario::handles("") {
            self.ids.entry(name).or_insert_with(|| place.to_string());
        }
        for url in self.h.take_opened() {
            self.opened.entry(url).or_insert_with(|| place.to_string());
        }
    }

    fn reset(&mut self) {
        self.h = fresh(self.lang);
        self.fresh += 1;
    }

    /// Escape out of whatever a click opened; a fresh app when that did not bring `want` back.
    fn back_to(&mut self, want: &str, tab: Option<&str>) {
        for _ in 0..2 {
            self.h.step(&json!({"key": "Escape"})).ok();
        }
        if fingerprint(&mut self.h) != want {
            self.reset();
            if let Some(tab) = tab {
                self.click(&json!({"handle": tab}), "tab");
            }
        }
    }

    fn click(&mut self, at: &serde_json::Value, place: &str) {
        self.click_with(at, place, None);
    }

    fn click_with(&mut self, at: &serde_json::Value, place: &str, button: Option<&str>) {
        self.clicks += 1;
        let mut step = json!({"click": at});
        if let Some(b) = button {
            step["button"] = json!(b);
        }
        self.h.step(&step).ok();
        self.h.frames(2);
        self.look(place);
    }

    /// A tab: its buttons' tooltips, its panels' drop-downs, then every widget a click reaches.
    fn tab(&mut self, tab: &str) {
        self.reset();
        self.click(&json!({"handle": tab}), tab);
        for (name, _) in scenario::handles("toolbar:") {
            if self.clicks > MAX_CLICKS {
                return;
            }
            let Some(at) = scenario::handle_at(&name) else { continue };
            let before: BTreeSet<String> = shown(&self.h).into_iter().collect();
            self.h.step(&json!({"move": [at.x, at.y]})).ok();
            // Until its tooltip shows (at once here; at most the default delay).
            for _ in 0..40 {
                self.h.frame();
                self.look(&format!("{tab} › tooltip {name}"));
                if shown(&self.h).iter().any(|t| !before.contains(t)) {
                    break;
                }
            }
        }
        let base = fingerprint(&mut self.h);
        for (name, _) in scenario::handles("panel:") {
            self.click(&json!({"handle": name}), &format!("{tab} › {name}"));
            self.back_to(&base, Some(tab));
        }
        self.everything(tab, &base);
    }

    /// Every clickable widget not clicked yet, and one level down every widget its click showed.
    fn everything(&mut self, place: &str, base: &str) {
        let before: BTreeSet<String> = clickables(&self.h).into_iter().map(|(k, _)| k).collect();
        for (key, at) in clickables(&self.h) {
            if self.clicks > MAX_CLICKS || key.starts_with("Tab") || !self.clicked.insert(key.clone()) {
                continue;
            }
            let here = format!("{place} › {key}");
            self.click(&json!(at), &here);
            let shown_now = clickables(&self.h);
            for (inner, at2) in shown_now.into_iter().filter(|(k, _)| !before.contains(k)) {
                // Once anywhere: a dialog's OK is the same button in every dialog.
                if self.clicks > MAX_CLICKS || !self.clicked.insert(inner.clone()) {
                    continue;
                }
                self.click(&json!(at2), &format!("{here} › {inner}"));
                self.back_to(base, Some(place).filter(|p| p.starts_with("tab:")));
                self.click(&json!(at), &here);
            }
            self.back_to(base, Some(place).filter(|p| p.starts_with("tab:")));
        }
    }

    /// Right-click menus where a user opens them, and every item in them.
    fn context_menus(&mut self) {
        self.reset();
        let mut spots: Vec<(String, [f64; 2])> = Vec::new();
        let viewport = self.h.call("ui.inspect", json!({}))["result"]["viewport"].clone();
        if let [Some(l), Some(t), Some(w), Some(hh)] = [0, 1, 2, 3].map(|i| viewport[i].as_f64()) {
            spots.push(("model".into(), [l + w / 2.0, t + hh / 2.0]));
            spots.push(("empty space".into(), [l + 30.0, t + hh - 30.0]));
        }
        for (name, p) in scenario::handles("row:").into_iter().take(6) {
            spots.push((name, [f64::from(p.x), f64::from(p.y)]));
        }
        if let Some(r) = scenario::rect_of("toolbar") {
            spots.push(("ribbon".into(), [f64::from(r.right()) - 20.0, f64::from(r.bottom()) - 8.0]));
        }
        let base = fingerprint(&mut self.h);
        for (name, at) in spots {
            let place = format!("right-click {name}");
            let before: BTreeSet<String> = clickables(&self.h).into_iter().map(|(k, _)| k).collect();
            self.click_with(&json!(at), &place, Some("right"));
            let items: Vec<_> = clickables(&self.h).into_iter().filter(|(k, _)| !before.contains(k)).collect();
            for (item, at2) in items {
                if self.clicks > MAX_CLICKS {
                    break;
                }
                self.click(&json!(at2), &format!("{place} › {item}"));
                self.back_to(&base, None);
                self.click_with(&json!(at), &place, Some("right"));
            }
            self.back_to(&base, None);
        }
    }

    /// Command search: the queries typed into the ribbon's box. What the box says back with the
    /// query in it (the query itself, "no command matches “…”") is the walk's own text, so a
    /// query nothing matches is typed first to learn those lines.
    fn search(&mut self) {
        const NOTHING: &str = "qzxqzx";
        let echoes: Vec<String> = self.typed(NOTHING).into_iter().filter(|t| t.contains(NOTHING)).collect();
        for q in QUERIES {
            let place = format!("search “{q}”");
            let own: Vec<String> = echoes.iter().map(|e| e.replace(NOTHING, q)).collect();
            for text in self.typed(q).into_iter().filter(|t| !own.contains(t)) {
                self.seen.entry(text).or_insert_with(|| place.clone());
            }
            self.look_ids(&place);
        }
    }

    /// The texts shown with `q` typed into the ribbon's search box.
    fn typed(&mut self, q: &str) -> Vec<String> {
        self.reset();
        self.click(&json!({"handle": "ribbon:search"}), "search");
        self.h.step(&json!({"text": q})).ok();
        self.h.frames(3);
        shown(&self.h)
    }

    /// What command search offers (every command it finds by its own id) and every key bound to a
    /// command the host refuses.
    fn offered(&mut self) {
        let app = &self.h.app;
        // Labels in the walk's language, as the box shows them.
        let _language = solidcraft_ui_text::scope(self.lang);
        for spec in solidcraft_engine::command_specs() {
            let rows = solidcraft_ui_egui::shortcut_box::search(app, spec.id);
            if let Some(row) = rows.iter().find(|r| r.0 == spec.id) {
                let place = format!("search offers {}", spec.id);
                self.ids.entry(format!("command:{}", spec.id)).or_insert_with(|| place.clone());
                self.seen.entry(row.1.to_string()).or_insert(place);
            }
            if solidcraft_ui_egui::brand::refused(spec.id).is_some() && !solidcraft_ui_egui::keymap::keys(app, spec.id).is_empty() {
                self.keyed.push(spec.id.to_string());
            }
        }
    }

    /// An error on the status line (its toast and actions) and a plain one.
    fn toasts(&mut self) {
        self.reset();
        self.h.app.set_status("Extrude failed: the profile is open", true);
        self.h.frames(3);
        self.look("error toast");
        self.reset();
        self.h.app.set_status("Saved", false);
        self.h.frames(3);
        self.look("status toast");
    }

    /// Every `ui.*` command started: windows and panels a key or search opens (not those the host
    /// refuses: nothing a user does starts them).
    fn ui_commands(&mut self) {
        let ids: Vec<&str> = solidcraft_engine::command_specs()
            .iter()
            .map(|c| c.id)
            .filter(|id| id.starts_with("ui.") && solidcraft_ui_egui::brand::refused(id).is_none())
            .collect();
        for id in ids {
            if self.clicks > MAX_CLICKS {
                break;
            }
            self.reset();
            self.h.app.start(id);
            self.h.frames(4);
            self.look(&format!("start {id}"));
        }
    }

    fn findings(&self) -> Vec<String> {
        let allowed = |s: &str, place: &str| {
            let s = s.trim();
            let last = s.rsplit(':').next().unwrap_or(s);
            ALLOW.iter().any(|(a, at, _)| (*a == s || *a == last) && place.contains(at))
                || ALLOW_ID_PREFIXES.iter().any(|(prefix, _)| s.starts_with(prefix))
        };
        let hit = |s: &str, needles: &[&str]| {
            let l = s.to_lowercase();
            needles.iter().find(|n| l.contains(*n)).map(|n| n.to_string())
        };
        let whole = |s: &str| {
            let l = s.trim().trim_end_matches('…').trim().to_lowercase();
            EXACT.iter().find(|n| **n == l).map(|n| n.to_string())
        };
        let mut out = Vec::new();
        for (text, place) in &self.seen {
            if let Some(n) = hit(text, BRAND).or_else(|| hit(text, ENTRIES)).or_else(|| whole(text))
                && !allowed(text, place)
            {
                out.push(format!("text “{text}” ({n}) at {place}"));
            }
        }
        for (id, place) in &self.ids {
            if let Some(n) = hit(id, ENTRY_IDS).or_else(|| hit(id, BRAND))
                && !allowed(id, place)
            {
                out.push(format!("id {id} ({n}) at {place}"));
            }
        }
        for (url, place) in &self.opened {
            if !allowed(url, place) {
                out.push(format!("opened {url} at {place}"));
            }
        }
        out.extend(self.keyed.iter().map(|id| format!("refused {id} still has a key")));
        out
    }

    fn write_report(&self, findings: &[String]) {
        let Some(dir) = std::env::var_os("BRAND_SCAN_OUT") else { return };
        let mut s = String::new();
        let _ = writeln!(s, "# findings ({})", findings.len());
        findings.iter().for_each(|f| _ = writeln!(s, "{f}"));
        let _ = writeln!(s, "\n# opened URLs");
        self.opened.iter().for_each(|(u, p)| _ = writeln!(s, "{u}\t{p}"));
        let _ = writeln!(s, "\n# ids (handles, offered commands)");
        self.ids.iter().for_each(|(i, p)| _ = writeln!(s, "{i}\t{p}"));
        let _ = writeln!(s, "\n# shown texts");
        self.seen.iter().for_each(|(t, p)| _ = writeln!(s, "{}\t{p}", t.replace('\n', "⏎")));
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join(format!("brand-scan-{}.txt", self.lang)), s);
    }
}

/// The host's editor in `lang`, with a body to act on. Save asks are answered in a staging folder
/// of the walk's own, as the host answers them (the scenario's folder is named after SolidCraft).
fn fresh(lang: &str) -> Harness {
    let mut h = Harness::new();
    host::editor(&mut h.app);
    let staging = std::env::temp_dir().join(format!("sz-brand-scan-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&staging);
    h.app.services.pick_save = Some(Box::new(move |name, _| Box::pin(std::future::ready(Some(staging.join(name).to_string_lossy().into_owned())))));
    h.app.preferences.language = lang.to_string();
    // Tooltips at once: the walk hovers every ribbon button.
    h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
    h.app.run("solid.box", json!({"length": 40, "width": 30, "height": 20})).expect("a body");
    h.frames(4);
    h
}

/// Every text the last frame painted (tooltips and toasts included) and every accessible label.
fn shown(h: &Harness) -> Vec<String> {
    fn painted(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| painted(s, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    h.shapes.iter().for_each(|s| painted(&s.shape, &mut out));
    if let Some(tree) = h.output.accesskit_update.as_ref() {
        for (_, n) in &tree.nodes {
            out.extend([n.label(), n.value()].into_iter().flatten().map(str::to_string));
        }
    }
    out.retain(|t| !t.trim().is_empty());
    out
}

/// Widgets whose click edits a value rather than leading anywhere: not clicked.
const VALUES: &[&str] = &[
    "TextInput",
    "MultilineTextInput",
    "SearchInput",
    "NumberInput",
    "PasswordInput",
    "SpinButton",
    "Slider",
    "ScrollBar",
    "CheckBox",
    "RadioButton",
    "Switch",
];

/// The widgets a click can reach: a key (role, label, or place for an unlabelled one) and centre.
fn clickables(h: &Harness) -> Vec<(String, [f64; 2])> {
    let Some(tree) = h.output.accesskit_update.as_ref() else { return Vec::new() };
    let mut out = Vec::new();
    for (_, n) in &tree.nodes {
        let Some(b) = n.bounds().filter(|b| b.x1 > b.x0 && b.y1 > b.y0) else { continue };
        if !n.supports_action(Action::Click) || VALUES.contains(&format!("{:?}", n.role()).as_str()) {
            continue;
        }
        let at = [(b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0];
        let label = n.label().or(n.value()).unwrap_or_default().trim().to_string();
        let key = if label.is_empty() { format!("{:?}@{:.0},{:.0}", n.role(), at[0], at[1]) } else { format!("{:?}:{label}", n.role()) };
        out.push((key, at));
    }
    out
}

/// What the UI is showing, to tell whether Escape brought it back: its clickable widgets and the
/// design's revision.
fn fingerprint(h: &mut Harness) -> String {
    let mut keys: Vec<String> = clickables(h).into_iter().map(|(k, _)| k).collect();
    keys.sort();
    keys.dedup();
    format!("{}|{}", h.call("document.revision", json!({}))["result"], keys.join("\n"))
}
