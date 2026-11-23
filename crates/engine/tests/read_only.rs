//! Host read-only mode (`solidcraft_engine::read_only`) through `Session::execute`. In a test
//! binary of its own: the switch is process-wide, and turning it on beside other tests would
//! refuse their commands too.

use serde_json::json;
use solidcraft_engine::Session;

/// A change is undone and refused with the host's reason, saving and undo are refused, reading
/// still works, and the design is exactly as it was; turned off, changes work again.
#[test]
fn read_only_keeps_the_design_and_says_why() {
    let mut s = Session::default();
    s.execute("solid.box", &json!({"length": 20, "width": 20, "height": 10})).unwrap();
    let before = s.design_revision();
    let doc = (*s.doc).clone();

    solidcraft_engine::read_only::set(Some("授权已到期".into()));
    let changed = s.execute("solid.box", &json!({"length": 5, "width": 5, "height": 5, "operation": "new"}));
    let undone = s.execute("edit.undo", &json!({}));
    let saved = s.execute("file.save", &json!({}));
    let looked = s.execute("document.inspect", &json!({}));
    let same = *s.doc == doc;
    solidcraft_engine::read_only::set(None);

    for refused in [changed, undone, saved] {
        assert!(refused.unwrap_err().to_string().contains("授权已到期"));
    }
    assert!(looked.is_ok(), "{looked:?}");
    // The content, not the revision: the error path's refresh may copy the document (lib.rs, `crash_outcome`).
    assert!(same, "the design changed under read-only");
    s.execute("solid.box", &json!({"length": 5, "width": 5, "height": 5, "operation": "new"})).unwrap();
    assert_ne!(s.design_revision(), before);
}
