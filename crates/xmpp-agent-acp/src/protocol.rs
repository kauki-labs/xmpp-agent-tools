//! The pure part: decode JSON-RPC lines and `session/update` notifications, build requests.

use std::fmt;
use std::path::Path;

use serde_json::Value;

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

pub fn parse_message(_line: &str) -> Result<Incoming, Error> {
    Err(Error::Protocol("todo".into()))
}

pub fn decode_update(_params: &Value) -> Option<AgentEvent> {
    None
}

pub fn decode_permission(_id: u64, _params: &Value) -> Result<AgentEvent, Error> {
    Err(Error::Protocol("todo".into()))
}

pub fn stop_reason(_result: &Value) -> String {
    String::new()
}

pub fn initialize_request(_id: u64) -> Value {
    Value::Null
}

pub fn new_session_request(_id: u64, _cwd: &Path) -> Value {
    Value::Null
}

pub fn load_session_request(_id: u64, _session_id: &str, _cwd: &Path) -> Value {
    Value::Null
}

pub fn prompt_request(_id: u64, _session_id: &str, _text: &str) -> Value {
    Value::Null
}

pub fn cancel_notification(_session_id: &str) -> Value {
    Value::Null
}

pub fn permission_response(_id: u64, _option_id: Option<&str>) -> Value {
    Value::Null
}

pub fn error_response(_id: u64, _code: i64, _message: &str) -> Value {
    Value::Null
}

#[cfg(test)]
mod tests;
