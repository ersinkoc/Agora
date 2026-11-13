//! JSON-RPC 2.0 envelope for the adapter protocol (docs/specs/adapter-protocol.md, ADR-0010).
//!
//! Adapters speak newline-delimited JSON-RPC over stdio. `params` and `result` are proto3-JSON encodings of the
//! messages in `schema/agora/v1/adapter.proto`; this module only handles the envelope and the method table.

use serde_json::{json, Value};

use crate::error::{Error, ErrorCode};

/// Application-level error code used in JSON-RPC error objects; the detail is in `data` as `{code, params}`.
pub const APP_ERROR: i64 = -32000;
pub const METHOD_NOT_FOUND: i64 = -32601;

/// Who sends a method, and whether it expects a response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    NodeToAdapter,
    AdapterToNode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Request,
    Notification,
}

/// One row of the method table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MethodInfo {
    pub name: &'static str,
    pub direction: Direction,
    pub kind: Kind,
    /// Message name (in `agora.v1`) of `params`.
    pub params: &'static str,
    /// Message name of `result`, for requests that return data.
    pub result: Option<&'static str>,
}

use Direction::{AdapterToNode as A2N, NodeToAdapter as N2A};
use Kind::{Notification as Note, Request as Req};

const fn m(
    name: &'static str,
    direction: Direction,
    kind: Kind,
    params: &'static str,
    result: Option<&'static str>,
) -> MethodInfo {
    MethodInfo {
        name,
        direction,
        kind,
        params,
        result,
    }
}

/// Every method of the adapter protocol. Unknown methods must be ignored (notifications) or answered with
/// `METHOD_NOT_FOUND` (requests); they are never fatal.
pub const METHODS: &[MethodInfo] = &[
    m(
        "initialize",
        N2A,
        Req,
        "InitializeParams",
        Some("InitializeResult"),
    ),
    m(
        "session/start",
        N2A,
        Req,
        "SessionStartParams",
        Some("SessionStartResult"),
    ),
    m("session/stop", N2A, Req, "SessionStopParams", None),
    m("shutdown", N2A, Req, "Empty", None),
    m("session/prompt", N2A, Req, "SessionPromptParams", None),
    m("session/cancel", N2A, Req, "SessionCancelParams", None),
    m("approval/resolve", N2A, Req, "ApprovalResolveParams", None),
    m("mesh/deliver", N2A, Req, "MeshDeliverParams", None),
    // ADR-0058
    m(
        "session/set_option",
        N2A,
        Req,
        "SessionSetOptionParams",
        None,
    ),
    m("state/update", A2N, Note, "StateUpdateParams", None),
    m("chat/append", A2N, Note, "ChatAppendParams", None),
    m("approval/request", A2N, Note, "ApprovalRequestParams", None),
    m("usage/report", A2N, Note, "UsageReportParams", None),
    m("session/reported", A2N, Note, "SessionReportedParams", None),
    m("log", A2N, Note, "LogParams", None),
    // ADR-0058
    m("session/options", A2N, Note, "SessionOptionsParams", None),
];

pub fn method(name: &str) -> Option<&'static MethodInfo> {
    METHODS.iter().find(|m| m.name == name)
}

/// A decoded line.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: Value,
        result: Value,
    },
    ErrorResponse {
        id: Value,
        code: i64,
        data: Option<Value>,
    },
}

fn invalid(field: &str) -> Error {
    Error::new(ErrorCode::ProtocolInvalidMessage).with("field", field)
}

/// Parse one line of input. Unknown extra fields are ignored.
pub fn parse_line(line: &str) -> Result<Message, Error> {
    let v: Value = serde_json::from_str(line).map_err(|_| invalid("json"))?;
    let obj = v.as_object().ok_or_else(|| invalid("message"))?;
    if obj.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(invalid("jsonrpc"));
    }
    let id = obj.get("id").cloned();
    if let Some(method) = obj.get("method") {
        let method = method
            .as_str()
            .ok_or_else(|| invalid("method"))?
            .to_string();
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        return Ok(match id {
            Some(id) if id.is_string() || id.is_number() => Message::Request { id, method, params },
            Some(_) => return Err(invalid("id")),
            None => Message::Notification { method, params },
        });
    }
    let id = id.ok_or_else(|| invalid("id"))?;
    if let Some(err) = obj.get("error") {
        let code = err
            .get("code")
            .and_then(Value::as_i64)
            .ok_or_else(|| invalid("error.code"))?;
        return Ok(Message::ErrorResponse {
            id,
            code,
            data: err.get("data").cloned(),
        });
    }
    let result = obj
        .get("result")
        .cloned()
        .ok_or_else(|| invalid("result"))?;
    Ok(Message::Response { id, result })
}

/// Serialize to one line (no trailing newline).
pub fn to_line(msg: &Message) -> String {
    let v = match msg {
        Message::Request { id, method, params } => {
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
        }
        Message::Notification { method, params } => {
            json!({"jsonrpc": "2.0", "method": method, "params": params})
        }
        Message::Response { id, result } => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Message::ErrorResponse { id, code, data } => {
            // JSON-RPC requires a `message`; it is a fixed diagnostic string, never user-facing text.
            let mut e = json!({"code": code, "message": "error"});
            if let Some(d) = data {
                e["data"] = d.clone();
            }
            json!({"jsonrpc": "2.0", "id": id, "error": e})
        }
    };
    v.to_string()
}

/// An application error carrying our `{code, params}` in `data`.
pub fn app_error(id: Value, err: &Error) -> Message {
    let params: serde_json::Map<String, Value> = err
        .params
        .iter()
        .map(|(k, v)| {
            let v = match v {
                crate::error::ParamValue::Str(s) => json!(s),
                crate::error::ParamValue::Int(i) => json!(i),
            };
            (k.clone(), v)
        })
        .collect();
    Message::ErrorResponse {
        id,
        code: APP_ERROR,
        data: Some(json!({"code": err.code.as_str(), "params": params})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_request_notification_response_and_error() {
        assert_eq!(
            parse_line(
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"x":1},"extra":true}"#
            ),
            Ok(Message::Request {
                id: json!(1),
                method: "initialize".into(),
                params: json!({"x": 1})
            })
        );
        assert_eq!(
            parse_line(r#"{"jsonrpc":"2.0","method":"log"}"#),
            Ok(Message::Notification {
                method: "log".into(),
                params: Value::Null
            })
        );
        assert_eq!(
            parse_line(r#"{"jsonrpc":"2.0","id":"a","result":{}}"#),
            Ok(Message::Response {
                id: json!("a"),
                result: json!({})
            })
        );
        assert_eq!(
            parse_line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"m"}}"#),
            Ok(Message::ErrorResponse {
                id: json!(2),
                code: METHOD_NOT_FOUND,
                data: None
            })
        );
    }

    #[test]
    fn rejects_malformed_lines_with_invalid_message() {
        for bad in [
            "",
            "not json",
            "[]",
            r#"{"id":1,"method":"x"}"#,
            r#"{"jsonrpc":"1.0","method":"x"}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"x"}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","method":5}"#,
        ] {
            assert_eq!(
                parse_line(bad).map_err(|e| e.code),
                Err(ErrorCode::ProtocolInvalidMessage),
                "{bad}"
            );
        }
    }

    #[test]
    fn lines_round_trip() {
        let msgs = [
            Message::Request {
                id: json!(7),
                method: "session/start".into(),
                params: json!({"a": [1, 2]}),
            },
            Message::Notification {
                method: "state/update".into(),
                params: json!({}),
            },
            Message::Response {
                id: json!("x"),
                result: json!({"sessionId": "s"}),
            },
            app_error(
                json!(3),
                &Error::new(ErrorCode::AgentKindUnknown).with("kind", "nope"),
            ),
        ];
        for m in msgs {
            assert_eq!(parse_line(&to_line(&m)), Ok(m));
        }
    }

    #[test]
    fn app_errors_carry_code_and_params_in_data() {
        let m = app_error(
            json!(1),
            &Error::new(ErrorCode::FilesTooLarge)
                .with("path", "a")
                .with("limit", 5),
        );
        let Message::ErrorResponse { code, data, .. } = m else {
            panic!("expected an error response");
        };
        assert_eq!(code, APP_ERROR);
        assert_eq!(
            data,
            Some(json!({"code": "files.too_large", "params": {"limit": 5, "path": "a"}}))
        );
    }

    #[test]
    fn method_table_matches_the_spec() {
        let spec = include_str!("../../../docs/specs/adapter-protocol.md");
        for m in METHODS.iter().filter(|m| {
            m.name != "initialize"
                && m.name != "shutdown"
                && m.name != "session/start"
                && m.name != "session/stop"
        }) {
            assert!(
                spec.contains(&format!("`{}`", m.name)),
                "{} is not in the spec",
                m.name
            );
        }
        assert_eq!(
            method("state/update").map(|m| m.kind),
            Some(Kind::Notification)
        );
        assert_eq!(method("nope"), None);
    }
}
