//! The pure part: decode JSON-RPC lines and `session/update` notifications, build requests.

use std::fmt;
use std::path::Path;

use serde_json::{Value, json};

/// What an agent tells the client during a turn.
#[derive(Debug, Clone, PartialEq)]
pub enum AgentEvent {
    Text(String),
    Thought(String),
    ToolCall {
        id: String,
        title: String,
        kind: String,
        raw_input: Value,
    },
    ToolUpdate {
        id: String,
        status: String,
        content: Option<String>,
    },
    /// The agent waits for [`crate::AcpSession::respond_permission`].
    Permission {
        request_id: u64,
        tool: String,
        options: Vec<PermOption>,
    },
    Commands(Vec<String>),
    Done {
        stop_reason: String,
    },
    Exited(String),
}

/// One choice of a permission request; `kind` is `allow_once`, `allow_always`, `reject_once` or `reject_always`.
#[derive(Debug, Clone, PartialEq)]
pub struct PermOption {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// A message that is not valid JSON-RPC or lacks a field this adapter needs.
    Protocol(String),
    /// The agent answered a request with an error.
    Rpc {
        code: i64,
        message: String,
    },
    /// The agent process is gone.
    Exited,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Protocol(m) => write!(f, "protocol: {m}"),
            Error::Rpc { code, message } => write!(f, "agent error {code}: {message}"),
            Error::Exited => write!(f, "the agent has exited"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

/// A decoded line from the agent.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Response {
        id: u64,
        result: Result<Value, (i64, String)>,
    },
    Notification {
        method: String,
        params: Value,
    },
    Request {
        id: u64,
        method: String,
        params: Value,
    },
}

/// Decode one line. Ids of requests from the agent must be unsigned integers: the permission reply is keyed by them.
pub fn parse_message(line: &str) -> Result<Incoming, Error> {
    let v: Value =
        serde_json::from_str(line).map_err(|e| Error::Protocol(format!("not JSON: {e}")))?;
    let id = v.get("id").filter(|i| !i.is_null());
    let method = v.get("method").and_then(Value::as_str);
    let params = v.get("params").cloned().unwrap_or(Value::Null);
    match (id, method) {
        (Some(id), Some(method)) => {
            let id = id
                .as_u64()
                .ok_or_else(|| Error::Protocol(format!("request id is not a number: {id}")))?;
            Ok(Incoming::Request {
                id,
                method: method.into(),
                params,
            })
        }
        (None, Some(method)) => Ok(Incoming::Notification {
            method: method.into(),
            params,
        }),
        (Some(id), None) => {
            let id = id
                .as_u64()
                .ok_or_else(|| Error::Protocol(format!("response id is not a number: {id}")))?;
            let result = match v.get("error") {
                Some(e) => Err((
                    e.get("code").and_then(Value::as_i64).unwrap_or(0),
                    e.get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                )),
                None => Ok(v
                    .get("result")
                    .cloned()
                    .ok_or_else(|| Error::Protocol("response without result or error".into()))?),
            };
            Ok(Incoming::Response { id, result })
        }
        (None, None) => Err(Error::Protocol("neither id nor method".into())),
    }
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(String::from)
}

/// Decode the `params` of a `session/update` notification; kinds the client does not use give `None`.
pub fn decode_update(params: &Value) -> Option<AgentEvent> {
    let u = params.get("update")?;
    let chunk_text = || str_at(u.get("content")?, "text");
    match u.get("sessionUpdate")?.as_str()? {
        "agent_message_chunk" => chunk_text().map(AgentEvent::Text),
        "agent_thought_chunk" => chunk_text().map(AgentEvent::Thought),
        "tool_call" => Some(AgentEvent::ToolCall {
            id: str_at(u, "toolCallId")?,
            title: str_at(u, "title").unwrap_or_default(),
            kind: str_at(u, "kind").unwrap_or_else(|| "other".into()),
            raw_input: u.get("rawInput").cloned().unwrap_or(Value::Null),
        }),
        "tool_call_update" => {
            let texts: Vec<&str> = u
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|c| c.get("type").and_then(Value::as_str) == Some("content"))
                .filter_map(|c| c.get("content")?.get("text")?.as_str())
                .collect();
            Some(AgentEvent::ToolUpdate {
                id: str_at(u, "toolCallId")?,
                status: str_at(u, "status").unwrap_or_default(),
                content: (!texts.is_empty()).then(|| texts.join("\n")),
            })
        }
        "available_commands_update" => Some(AgentEvent::Commands(
            u.get("availableCommands")?
                .as_array()?
                .iter()
                .filter_map(|c| str_at(c, "name"))
                .collect(),
        )),
        _ => None,
    }
}

/// Decode the `params` of a `session/request_permission` request received under JSON-RPC id `id`.
pub fn decode_permission(id: u64, params: &Value) -> Result<AgentEvent, Error> {
    let bad = |what: &str| Error::Protocol(format!("permission request: {what}"));
    let tool = params.get("toolCall").ok_or_else(|| bad("no toolCall"))?;
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no options"))?
        .iter()
        .map(|o| {
            Ok(PermOption {
                id: str_at(o, "optionId").ok_or_else(|| bad("option without optionId"))?,
                name: str_at(o, "name").unwrap_or_default(),
                kind: str_at(o, "kind").unwrap_or_default(),
            })
        })
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(AgentEvent::Permission {
        request_id: id,
        tool: str_at(tool, "title").unwrap_or_default(),
        options,
    })
}

/// The `stopReason` of a `session/prompt` result.
pub fn stop_reason(result: &Value) -> String {
    str_at(result, "stopReason").unwrap_or_else(|| "unknown".into())
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

pub fn initialize_request(id: u64) -> Value {
    request(
        id,
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {"name": "xmpp-agent-acp", "version": env!("CARGO_PKG_VERSION")},
        }),
    )
}

pub fn new_session_request(id: u64, cwd: &Path) -> Value {
    request(
        id,
        "session/new",
        json!({"cwd": cwd.to_string_lossy(), "mcpServers": []}),
    )
}

pub fn load_session_request(id: u64, session_id: &str, cwd: &Path) -> Value {
    request(
        id,
        "session/load",
        json!({"sessionId": session_id, "cwd": cwd.to_string_lossy(), "mcpServers": []}),
    )
}

pub fn prompt_request(id: u64, session_id: &str, text: &str) -> Value {
    request(
        id,
        "session/prompt",
        json!({"sessionId": session_id, "prompt": [{"type": "text", "text": text}]}),
    )
}

pub fn cancel_notification(session_id: &str) -> Value {
    json!({"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": session_id}})
}

/// The reply to a permission request: the chosen option, or `None` for cancelled.
pub fn permission_response(id: u64, option_id: Option<&str>) -> Value {
    let outcome = match option_id {
        Some(o) => json!({"outcome": "selected", "optionId": o}),
        None => json!({"outcome": "cancelled"}),
    };
    json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}})
}

pub fn error_response(id: u64, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(test)]
mod tests;
