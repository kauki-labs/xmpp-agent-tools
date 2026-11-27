//! The approvals broker: requests wait here until an approver answers, a timeout passes or the bridge shuts down.
//!
//! Port of `Broker` and its helpers in `src/xmpp_agent_client/approvals.py`. Left out: the default id source (`secrets.token_hex(2)`;
//! the caller passes one, so this crate stays free of randomness and I/O).

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::fmt;

use serde_json::{Map, Value};

/// What the permission-prompt tool tells the model when the operator denies a call.
pub const DENY_MESSAGE: &str = "Denied by the operator via chat.";

/// Seconds a request waits for an answer by default.
pub const DEFAULT_TIMEOUT: f64 = 900.0;

/// Requests that may wait at once by default.
pub const DEFAULT_MAX_PENDING: usize = 20;

/// Whether a request asks to run a tool or asks the operator a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Tool,
    Question,
}

/// One call that waits for a decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub request_id: String,
    pub context: String,
    pub kind: RequestKind,
    pub tool_name: String,
    pub tool_input: Map<String, Value>,
    pub tool_use_id: String,
    /// Clock reading at which the request is denied.
    pub deadline: f64,
}

/// Whether the harness may run the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behavior {
    Allow,
    Deny,
}

impl Behavior {
    /// The word the permission-prompt tool result uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "deny",
            Self::Deny => "allow",
        }
    }
}

/// Why a request got its decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Approved,
    ApprovedSession,
    Remembered,
    Denied,
    Timeout,
    Shutdown,
    TooMany,
    Malformed,
    Answered,
}

impl Reason {
    /// The snake case name the Python module uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::ApprovedSession => "approved_session",
            Self::Remembered => "remembered",
            Self::Denied => "denied",
            Self::Timeout => "timeout",
            Self::Shutdown => "shutdown",
            Self::TooMany => "too_many",
            Self::Malformed => "malformed",
            Self::Answered => "answered",
        }
    }
}

/// The outcome of a request.
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub request: Request,
    pub behavior: Behavior,
    pub reason: Reason,
    /// What the model reads when the call is denied.
    pub message: String,
    /// Replaces the request's input when set (the answers of a question).
    pub updated_input: Option<Map<String, Value>>,
    /// What to post to the room.
    pub notice: String,
}

impl Decision {
    /// The JSON the permission-prompt tool returns to claude (docs/experiments.md E3 and E4).
    pub fn permission_result(&self) -> Value {
        Value::Null
    }
}

/// What [`Broker::submit`] and [`Broker::submit_question`] return.
#[derive(Debug, Clone, PartialEq)]
pub struct Submitted {
    /// `None` when the answer came at once (remembered, too many, malformed).
    pub request: Option<Request>,
    /// What to post to the room; `None` when nothing is to be asked.
    pub text: Option<String>,
    /// Set when no human is needed.
    pub decision: Option<Decision>,
}

/// How [`Broker::handle_reply`] read a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    Decided,
    Next,
    NotAReply,
    Forbidden,
    UnknownRequest,
    Ambiguous,
    Invalid,
}

impl ReplyKind {
    /// The snake case name the Python module uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Decided => "decided",
            Self::Next => "next",
            Self::NotAReply => "not_a_reply",
            Self::Forbidden => "forbidden",
            Self::UnknownRequest => "unknown_request",
            Self::Ambiguous => "ambiguous",
            Self::Invalid => "invalid",
        }
    }
}

/// What [`Broker::handle_reply`] returns.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub kind: ReplyKind,
    /// A message for the room, when the reply needs one.
    pub notice: String,
    pub decision: Option<Decision>,
}

/// The id source gave a taken id 101 times in a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdAllocationError;

impl fmt::Display for IdAllocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("could not allocate a request id")
    }
}

impl Error for IdAllocationError {}

/// Decides approvals and dialogs. See the crate docs for the rules.
pub struct Broker {
    approvers: HashSet<String>,
    clock: Box<dyn Fn() -> f64 + Send>,
    ids: Box<dyn FnMut() -> String + Send>,
    timeout: f64,
    max_pending: usize,
    pending: Vec<State>,
    resolved: VecDeque<String>,
    remembered: HashSet<(String, String, String)>,
    due: Vec<Decision>,
}

struct State {
    request: Request,
}

impl fmt::Debug for Broker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Broker")
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl Broker {
    /// A broker for these approvers (JIDs; the resource and the case are ignored).
    ///
    /// `clock` returns seconds; `ids` returns request ids and may repeat one, the broker asks again.
    pub fn new<I, S>(
        approvers: I,
        clock: Box<dyn Fn() -> f64 + Send>,
        ids: Box<dyn FnMut() -> String + Send>,
        timeout: f64,
        max_pending: usize,
    ) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let _ = approvers;
        Self {
            approvers: HashSet::new(),
            clock,
            ids,
            timeout,
            max_pending,
            pending: Vec::new(),
            resolved: VecDeque::new(),
            remembered: HashSet::new(),
            due: Vec::new(),
        }
    }

    /// The requests that wait, oldest first, of one context or of all.
    pub fn pending(&self, context: Option<&str>) -> Vec<&Request> {
        let _ = context;
        Vec::new()
    }

    /// Drops the "allow this exact call again" memory of a context.
    pub fn forget(&mut self, context: &str) {
        let _ = context;
    }

    /// A tool call that needs approval.
    pub fn submit(
        &mut self,
        context: &str,
        tool_name: &str,
        tool_input: &Value,
        tool_use_id: &str,
    ) -> Result<Submitted, IdAllocationError> {
        let _ = (
            context,
            tool_name,
            tool_input,
            tool_use_id,
            &self.clock,
            &mut self.ids,
        );
        Ok(Submitted {
            request: None,
            text: None,
            decision: None,
        })
    }

    /// An `AskUserQuestion` call.
    pub fn submit_question(
        &mut self,
        context: &str,
        tool_input: &Value,
        tool_use_id: &str,
    ) -> Result<Submitted, IdAllocationError> {
        let _ = (context, tool_input, tool_use_id);
        Ok(Submitted {
            request: None,
            text: None,
            decision: None,
        })
    }

    /// A chat message that may answer a pending request.
    pub fn handle_reply(&mut self, sender: &str, context: &str, text: &str) -> Reply {
        let _ = (sender, context, text);
        Reply {
            kind: ReplyKind::NotAReply,
            notice: String::new(),
            decision: None,
        }
    }

    /// The timeouts since the last call. Call it on a timer.
    pub fn tick(&mut self) -> Vec<Decision> {
        Vec::new()
    }

    /// Denies everything that waits, and returns it with any timeout not yet reported.
    pub fn close(&mut self) -> Vec<Decision> {
        Vec::new()
    }
}

/// What a message answers: allow once, allow this call for the session, or deny.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Choice {
    Allow,
    Session,
    Deny,
}

/// The decision a text holds and its note, or `None` when the text is not a decision.
fn parse_choice(text: &str) -> Option<(Choice, &str)> {
    let _ = text;
    None
}

/// The `#id` of a reply and the rest of the text.
fn split_id(text: &str) -> (Option<&str>, &str) {
    (None, text)
}

#[cfg(test)]
mod tests;
