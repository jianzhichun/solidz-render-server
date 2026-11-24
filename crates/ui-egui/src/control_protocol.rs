//! What SolidCraft's control requests mean to the shared control server (`solidcraft_control`):
//! the one [`Protocol`] every host of this UI answers with, so a client bridged to any of them
//! (the desktop app, an embedding host's offscreen render server) sees the same app: the same
//! summaries and timeouts, the same document name, change event and revision method, and the
//! same capabilities. Moved here unchanged from the desktop app's `control_server.rs`.

use serde_json::{Value, json};
use solidcraft_control::server::{Protocol, Summary};
use solidcraft_engine::control_pending;

/// What SolidCraft's requests mean to the control server: commands by their ids, batches and
/// scripts by their steps, and which ones read, write a file or change the design.
pub struct AppProtocol;

impl Protocol for AppProtocol {
    fn summary(&self, method: &str, p: &Value) -> Summary {
        let command = match method {
            "engine.execute" | "command" => p.get("command").and_then(Value::as_str).and_then(solidcraft_engine::find_command).map(|c| c.id),
            _ => None,
        };
        let steps = match method {
            "engine.batch" | "engine.script" => p.get("commands").and_then(Value::as_array).map(Vec::len),
            _ => None,
        };
        Summary {
            what: control_pending::describe(method, p),
            effect: control_pending::effect(method, p),
            command,
            steps,
            long: method == "engine.batch",
        }
    }

    fn document(&self) -> &'static str {
        "design"
    }

    fn change_event(&self) -> &'static str {
        "design.changed"
    }

    fn revision_method(&self) -> &'static str {
        "document.revision"
    }

    fn docs(&self) -> &'static str {
        "docs/control-protocol.md"
    }

    fn stop_on_disconnect(&self, method: &str) -> bool {
        method == "control.joint_drive"
    }

    /// A plugin package job holds `plugin.install` and `plugin.remove` for every connection.
    fn background(&self) -> Vec<Value> {
        crate::plugin_install::running_jobs()
    }

    /// Advertise parameter checks, installed plugin tools and interactive desktop joint drive.
    fn capabilities(&self) -> Value {
        json!({
            "param_check": solidcraft_engine::schema::ParamCheck::ALL.map(|l| l.name()),
            "plugin_tools": true,
            "joint_drive": true,
            "joint_pose": true,
        })
    }
}
