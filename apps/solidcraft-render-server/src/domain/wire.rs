//! The wire protocol's messages (PROTOCOL.md, "Messages" and "Shared types"): one enum per
//! direction, field names exactly as the embedding host's TypeScript contract spells them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Moves with any change an old peer would misread (`hello.v`).
pub const PROTOCOL_VERSION: u32 = 1;
/// Frames the host may have unacknowledged; one `ack` gives one back.
pub const INITIAL_CREDITS: u32 = 2;
/// Passes (and so frames) per second, at most.
pub const MAX_FRAMES_PER_SECOND: u32 = 60;
/// Largest frame side in device pixels; a bigger pane is drawn at a lower dpr.
pub const MAX_DEVICE_SIDE: u32 = 4096;

/// Render server → host.
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "k", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ServerMsg {
    Hello {
        v: u32,
        token: String,
        pid: u32,
        version: String,
        backend: Option<&'static str>,
        adapter: String,
        software: bool,
    },
    Ready {
        control_port: u16,
        w: u32,
        h: u32,
        dpr: f32,
        samples: u32,
    },
    Frame {
        seq: u64,
        input_seq: u64,
        w: u32,
        h: u32,
        dpr: f32,
        full: bool,
        rects: Vec<FrameRect>,
        timing: FrameTiming,
    },
    Cursor {
        icon: String,
    },
    Ime {
        ime: Option<ImeState>,
    },
    Copied {
        text: String,
    },
    AskSave {
        ask: u32,
        name: String,
        exts: Vec<String>,
    },
    AskOpen {
        ask: u32,
        exts: Vec<String>,
    },
    AskDone {
        ask: u32,
        ok: bool,
    },
    Bye {
        why: ByeWhy,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

/// Host → render server. `input.events` stay raw JSON here: each is read on its own
/// ([`super::input::parse_events`]), so one event a newer page sends cannot drop the rest.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "k", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum HostMsg {
    Input { seq: u64, events: Vec<Value> },
    Resize { w: u32, h: u32, dpr: f32 },
    Visible { visible: bool },
    Ack { seq: u64, decode_ms: f32 },
    Policy { read_only: Option<String> },
    Answer { ask: u32, path: Option<String> },
    Stop { why: String },
    Error { code: String, message: String },
}

/// The kinds the host may send (PROTOCOL.md, "Host → render server").
const HOST_KINDS: [&str; 8] = ["input", "resize", "visible", "ack", "policy", "answer", "stop", "error"];

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ByeWhy {
    Stop,
    HostGone,
    DeviceLost,
    Error,
}

/// The `error` codes this side sends; `version`, `token` and `order` are the host's to send.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    Malformed,
    TooLarge,
    UnknownKind,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Codec {
    Jpeg,
    Png,
    Rgba,
}

/// A changed rectangle in device pixels; its bytes are `payload[offset..offset + length]`.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub struct FrameRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub codec: Codec,
    pub ew: u32,
    pub eh: u32,
    pub offset: u32,
    pub length: u32,
}

/// Milliseconds this frame spent in each stage (durations only).
#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq)]
pub struct FrameTiming {
    pub pass: f32,
    pub paint: f32,
    pub readback: f32,
    pub diff: f32,
    pub encode: f32,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImePurpose {
    Normal,
    Password,
    Terminal,
}

/// Where a text field wants the IME, in CSS px of the pane: `[x, y, w, h]`.
#[derive(Clone, Copy, Debug, Serialize, PartialEq)]
pub struct ImeState {
    pub purpose: ImePurpose,
    pub rect: [f32; 4],
    pub cursor: [f32; 4],
    pub interrupt: bool,
}

/// Why a header the host sent was refused: the `error` to send back before closing.
#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: ErrorCode,
    pub message: String,
}

impl Refusal {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Refusal { code, message: message.into() }
    }

    pub fn into_msg(self) -> ServerMsg {
        ServerMsg::Error { code: self.code, message: self.message }
    }
}

/// Read one header from the host: a JSON object whose `k` is a kind this direction carries,
/// with that kind's fields.
pub fn parse_host(header: &[u8]) -> Result<HostMsg, Refusal> {
    let value: Value = serde_json::from_slice(header).map_err(|e| Refusal::new(ErrorCode::Malformed, format!("header is not JSON: {e}")))?;
    let kind = value
        .as_object()
        .ok_or_else(|| Refusal::new(ErrorCode::Malformed, "header is not a JSON object"))?
        .get("k")
        .and_then(Value::as_str)
        .ok_or_else(|| Refusal::new(ErrorCode::Malformed, "header has no `k`"))?;
    if !HOST_KINDS.contains(&kind) {
        return Err(Refusal::new(ErrorCode::UnknownKind, format!("unknown kind `{kind}`")));
    }
    let kind = kind.to_string();
    serde_json::from_value(value).map_err(|e| Refusal::new(ErrorCode::Malformed, format!("`{kind}`: {e}")))
}

/// The header bytes of `msg` (never fails: every field serialises).
pub fn header(msg: &ServerMsg) -> Vec<u8> {
    serde_json::to_vec(msg).unwrap_or_else(|e| format!(r#"{{"k":"error","code":"malformed","message":"{e}"}}"#).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn json_of(msg: &ServerMsg) -> Value {
        serde_json::from_slice(&header(msg)).unwrap()
    }

    #[test]
    fn server_messages_use_the_contract_names() {
        let frame = ServerMsg::Frame {
            seq: 3,
            input_seq: 7,
            w: 10,
            h: 20,
            dpr: 2.0,
            full: true,
            rects: vec![FrameRect { x: 0, y: 0, w: 10, h: 20, codec: Codec::Rgba, ew: 10, eh: 20, offset: 0, length: 800 }],
            timing: FrameTiming::default(),
        };
        let v = json_of(&frame);
        assert_eq!((&v["k"], &v["inputSeq"], &v["rects"][0]["codec"]), (&json!("frame"), &json!(7), &json!("rgba")));
        assert!(v["timing"]["readback"].is_number());
        let ready = json_of(&ServerMsg::Ready { control_port: 9, w: 1, h: 1, dpr: 1.0, samples: 4 });
        assert_eq!((&ready["k"], &ready["controlPort"]), (&json!("ready"), &json!(9)));
        assert_eq!(json_of(&ServerMsg::AskDone { ask: 1, ok: true })["k"], "ask_done");
        // Nullable fields are written, optional ones left out.
        assert_eq!(json_of(&ServerMsg::Ime { ime: None }), json!({"k": "ime", "ime": null}));
        assert_eq!(json_of(&ServerMsg::Bye { why: ByeWhy::HostGone, detail: None }), json!({"k": "bye", "why": "host-gone"}));
        let error = json_of(&Refusal::new(ErrorCode::UnknownKind, "x").into_msg());
        assert_eq!(error["code"], "unknown-kind");
    }

    #[test]
    fn host_messages_parse_by_kind() {
        let parse = |v: Value| parse_host(v.to_string().as_bytes());
        assert_eq!(parse(json!({"k": "ack", "seq": 4, "decodeMs": 1.5})), Ok(HostMsg::Ack { seq: 4, decode_ms: 1.5 }));
        assert_eq!(parse(json!({"k": "policy", "readOnly": null})), Ok(HostMsg::Policy { read_only: None }));
        assert_eq!(parse(json!({"k": "answer", "ask": 2, "path": "/s/a.step"})), Ok(HostMsg::Answer { ask: 2, path: Some("/s/a.step".into()) }));
        assert!(matches!(parse(json!({"k": "input", "seq": 1, "events": [{"e": "gone"}]})), Ok(HostMsg::Input { seq: 1, .. })));
    }

    #[test]
    fn bad_headers_are_refused_with_their_code() {
        let code = |bytes: &[u8]| parse_host(bytes).unwrap_err().code;
        assert_eq!(code(b"[1]"), ErrorCode::Malformed);
        assert_eq!(code(b"not json"), ErrorCode::Malformed);
        assert_eq!(code(br#"{"k":"frame"}"#), ErrorCode::UnknownKind);
        assert_eq!(code(br#"{"k":"resize","w":"wide"}"#), ErrorCode::Malformed);
    }
}
