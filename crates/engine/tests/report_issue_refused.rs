//! A host that takes problem reports its own way (`solidcraft_engine::report_issue::refuse`). In a
//! test binary of its own: the switch is process-wide.

use serde_json::json;
use solidcraft_engine::{Session, report_issue};

/// Refused, `diagnostics.report_issue` says why and queues nothing, even with an app attached;
/// not refused, it queues the request for the app as before.
#[test]
fn a_refused_report_is_never_queued() {
    let mut s = Session::default();
    report_issue::attach_host();
    report_issue::refuse(Some("这里不能发送问题报告"));
    let refused = s.execute("diagnostics.report_issue", &json!({"title": "Fillet fails"})).unwrap_err().to_string();
    assert!(refused.contains("这里不能发送问题报告"), "{refused}");
    assert!(report_issue::take_requests(s.uid).is_empty(), "nothing reaches the app");
    report_issue::refuse(None);
    s.execute("diagnostics.report_issue", &json!({"title": "Fillet fails"})).unwrap();
    assert_eq!(report_issue::take_requests(s.uid).len(), 1);
}
