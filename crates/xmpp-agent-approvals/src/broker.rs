//! The approvals broker: requests wait here until an approver answers, a timeout passes or the bridge shuts down.
//!
//! Port of `Broker` and its helpers in `src/xmpp_agent_client/approvals.py`. Left out: the default id source (`secrets.token_hex(2)`;
//! the caller passes one, so this crate stays free of randomness and I/O).

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::fmt;

use serde_json::{Map, Value, json};
use xmpp_agent_common::redact::{DEFAULT_MAX_CHARS, describe_tool, redact};

use crate::text::{is_space, lower, parse_numbers, split_first_word, strip, truthy};

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
            Self::Allow => "allow",
            Self::Deny => "deny",
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
        match self.behavior {
            Behavior::Allow => {
                let input = self
                    .updated_input
                    .as_ref()
                    .unwrap_or(&self.request.tool_input);
                json!({"behavior": "allow", "updatedInput": input})
            }
            Behavior::Deny => json!({"behavior": "deny", "message": self.message}),
        }
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
    /// Oldest first.
    pending: Vec<State>,
    /// The ids of the last 256 requests that were decided or timed out.
    resolved: VecDeque<String>,
    /// (context, tool name, canonical input) of the calls the approver allowed for the session.
    remembered: HashSet<(String, String, String)>,
    /// Timeouts found by a reply, waiting for the next [`Broker::tick`].
    due: Vec<Decision>,
}

/// How many resolved ids the broker remembers, to tell "decided" from "never pending".
const RESOLVED_CAPACITY: usize = 256;

/// How many times the broker checks an id before it gives up.
const ID_ATTEMPTS: usize = 100;

/// The longest note of a denial that reaches the model, in characters.
const NOTE_MAX_CHARS: usize = 300;

/// A request and, for a question, how far the dialog has come.
struct State {
    request: Request,
    questions: Vec<Question>,
    index: usize,
    answers: Map<String, Value>,
}

struct Question {
    text: String,
    options: Vec<AnswerOption>,
    multi_select: bool,
}

struct AnswerOption {
    label: String,
    /// Only a non-empty text: Python shows any other value that is true, and fails on it.
    description: Option<String>,
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
        Self {
            approvers: approvers
                .into_iter()
                .map(|approver| bare(approver.as_ref()))
                .collect(),
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
        self.pending
            .iter()
            .map(|state| &state.request)
            .filter(|request| context.is_none_or(|c| request.context == c))
            .collect()
    }

    /// Drops the "allow this exact call again" memory of a context.
    pub fn forget(&mut self, context: &str) {
        self.remembered
            .retain(|(remembered_context, _, _)| remembered_context != context);
    }

    /// A tool call that needs approval.
    ///
    /// An input that is not a JSON object counts as an empty one.
    ///
    /// # Errors
    ///
    /// [`IdAllocationError`] when the id source keeps giving ids that are taken.
    pub fn submit(
        &mut self,
        context: &str,
        tool_name: &str,
        tool_input: &Value,
        tool_use_id: &str,
    ) -> Result<Submitted, IdAllocationError> {
        let tool_input = object_of(tool_input);
        if tool_name != "ExitPlanMode"
            && self
                .remembered
                .contains(&call_key(context, tool_name, &tool_input))
        {
            let request = self.request(
                context,
                RequestKind::Tool,
                tool_name,
                tool_input,
                tool_use_id,
            )?;
            let decision = Decision {
                request,
                behavior: Behavior::Allow,
                reason: Reason::Remembered,
                message: String::new(),
                updated_input: None,
                notice: String::new(),
            };
            return Ok(Submitted {
                request: None,
                text: None,
                decision: Some(decision),
            });
        }
        if self.pending.len() >= self.max_pending {
            return self.refuse(
                context,
                RequestKind::Tool,
                tool_name,
                tool_input,
                tool_use_id,
                Reason::TooMany,
            );
        }
        let request = self.request(
            context,
            RequestKind::Tool,
            tool_name,
            tool_input,
            tool_use_id,
        )?;
        let text = format!(
            "Approval #{}: {}\nReply 1 to allow once, 2 to allow this exact call again in this session, 3 to deny, or 4 and a note to deny \
             with a reason. It is denied after {} min without an answer.",
            request.request_id,
            describe_tool(
                tool_name,
                &Value::Object(request.tool_input.clone()),
                DEFAULT_MAX_CHARS
            ),
            self.minutes(),
        );
        self.pending.push(State {
            request: request.clone(),
            questions: Vec::new(),
            index: 0,
            answers: Map::new(),
        });
        Ok(Submitted {
            request: Some(request),
            text: Some(text),
            decision: None,
        })
    }

    /// An `AskUserQuestion` call.
    ///
    /// A question that is not well formed is denied. An input that is not a JSON object counts as an empty one.
    ///
    /// # Errors
    ///
    /// [`IdAllocationError`] when the id source keeps giving ids that are taken.
    pub fn submit_question(
        &mut self,
        context: &str,
        tool_input: &Value,
        tool_use_id: &str,
    ) -> Result<Submitted, IdAllocationError> {
        let tool_input = object_of(tool_input);
        let Some(questions) = parse_questions(&tool_input) else {
            return self.refuse(
                context,
                RequestKind::Question,
                "AskUserQuestion",
                tool_input,
                tool_use_id,
                Reason::Malformed,
            );
        };
        if self.pending.len() >= self.max_pending {
            return self.refuse(
                context,
                RequestKind::Question,
                "AskUserQuestion",
                tool_input,
                tool_use_id,
                Reason::TooMany,
            );
        }
        let request = self.request(
            context,
            RequestKind::Question,
            "AskUserQuestion",
            tool_input,
            tool_use_id,
        )?;
        let state = State {
            request: request.clone(),
            questions,
            index: 0,
            answers: Map::new(),
        };
        let text = render_question(&state);
        self.pending.push(state);
        Ok(Submitted {
            request: Some(request),
            text: Some(text),
            decision: None,
        })
    }

    /// A chat message that may answer a pending request.
    ///
    /// The reply is matched to a request by `context`, and by `#id` when the text has one. Only an approver's answer decides.
    pub fn handle_reply(&mut self, sender: &str, context: &str, text: &str) -> Reply {
        self.expire();
        let (rid, body) = split_id(text);
        let is_approver = self.approvers.contains(&bare(sender));
        let in_context: Vec<usize> = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, state)| state.request.context == context)
            .map(|(index, _)| index)
            .collect();
        let index = match rid {
            Some(rid) => match self.pending.iter().position(|state| {
                state.request.request_id == rid && state.request.context == context
            }) {
                Some(index) => index,
                None => {
                    // "#123" in ordinary chat
                    if parse_choice(body).is_none() {
                        return reply(ReplyKind::NotAReply, String::new());
                    }
                    if !is_approver {
                        return reply(ReplyKind::Forbidden, String::new());
                    }
                    let gone = if self.resolved.iter().any(|resolved| resolved == rid) {
                        "is no longer pending (already decided or timed out)"
                    } else {
                        "is not pending here"
                    };
                    return reply(ReplyKind::UnknownRequest, format!("Request #{rid} {gone}."));
                }
            },
            None => match in_context.as_slice() {
                [] => return reply(ReplyKind::NotAReply, String::new()),
                [only] => *only,
                several => {
                    if parse_choice(body).is_none() && parse_numbers(strip(body)).is_none() {
                        return reply(ReplyKind::NotAReply, String::new());
                    }
                    if !is_approver {
                        return reply(ReplyKind::Forbidden, String::new());
                    }
                    let ids: Vec<String> = several
                        .iter()
                        .map(|index| format!("#{}", self.pending[*index].request.request_id))
                        .collect();
                    let notice = format!(
                        "Several requests are waiting ({}). Add the id to your answer, for example `1 {}`.",
                        ids.join(", "),
                        ids[0]
                    );
                    return reply(ReplyKind::Ambiguous, notice);
                }
            },
        };
        if !looks_like_answer(&self.pending[index], body) {
            return reply(ReplyKind::NotAReply, String::new());
        }
        if !is_approver {
            return reply(ReplyKind::Forbidden, String::new());
        }
        self.answer(index, body)
    }

    /// The timeouts since the last call. Call it on a timer.
    pub fn tick(&mut self) -> Vec<Decision> {
        self.expire();
        std::mem::take(&mut self.due)
    }

    /// Denies everything that waits, and returns it with any timeout not yet reported.
    pub fn close(&mut self) -> Vec<Decision> {
        let mut out = std::mem::take(&mut self.due);
        for state in std::mem::take(&mut self.pending) {
            let notice = format!(
                "Approval #{} was denied: the bridge is shutting down.",
                state.request.request_id
            );
            out.push(self.finish(
                state,
                Behavior::Deny,
                Reason::Shutdown,
                "The bridge shut down before the operator answered.",
                None,
                notice,
            ));
        }
        out
    }

    fn minutes(&self) -> i64 {
        (self.timeout / 60.0).round_ties_even() as i64
    }

    fn request(
        &mut self,
        context: &str,
        kind: RequestKind,
        tool_name: &str,
        tool_input: Map<String, Value>,
        tool_use_id: &str,
    ) -> Result<Request, IdAllocationError> {
        let mut request_id = (self.ids)();
        let mut found = false;
        for _ in 0..ID_ATTEMPTS {
            if !self.is_taken(&request_id) {
                found = true;
                break;
            }
            request_id = (self.ids)();
        }
        if !found {
            return Err(IdAllocationError);
        }
        Ok(Request {
            request_id,
            context: context.to_owned(),
            kind,
            tool_name: tool_name.to_owned(),
            tool_input,
            tool_use_id: tool_use_id.to_owned(),
            deadline: (self.clock)() + self.timeout,
        })
    }

    fn is_taken(&self, request_id: &str) -> bool {
        self.pending
            .iter()
            .any(|state| state.request.request_id == request_id)
            || self.resolved.iter().any(|resolved| resolved == request_id)
    }

    fn refuse(
        &mut self,
        context: &str,
        kind: RequestKind,
        tool_name: &str,
        tool_input: Map<String, Value>,
        tool_use_id: &str,
        reason: Reason,
    ) -> Result<Submitted, IdAllocationError> {
        let request = self.request(context, kind, tool_name, tool_input, tool_use_id)?;
        let notice = match reason {
            Reason::TooMany => "Too many approvals are waiting; the newest was denied.",
            _ => "The harness sent a question the bridge cannot show; denied.",
        };
        let decision = Decision {
            request,
            behavior: Behavior::Deny,
            reason,
            message: notice.to_owned(),
            updated_input: None,
            notice: notice.to_owned(),
        };
        Ok(Submitted {
            request: None,
            text: None,
            decision: Some(decision),
        })
    }

    /// Ends a request that left `pending`: remembers its id and builds the decision.
    fn finish(
        &mut self,
        state: State,
        behavior: Behavior,
        reason: Reason,
        message: &str,
        updated_input: Option<Map<String, Value>>,
        notice: String,
    ) -> Decision {
        self.resolved.push_back(state.request.request_id.clone());
        if self.resolved.len() > RESOLVED_CAPACITY {
            self.resolved.pop_front();
        }
        Decision {
            request: state.request,
            behavior,
            reason,
            message: message.to_owned(),
            updated_input,
            notice,
        }
    }

    fn expire(&mut self) {
        let now = (self.clock)();
        let (expired, waiting): (Vec<State>, Vec<State>) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|state| now >= state.request.deadline);
        self.pending = waiting;
        for state in expired {
            let notice = format!(
                "Approval #{} timed out after {} min and was denied.",
                state.request.request_id,
                self.minutes()
            );
            let decision = self.finish(
                state,
                Behavior::Deny,
                Reason::Timeout,
                "Timed out waiting for the operator via chat.",
                None,
                notice,
            );
            self.due.push(decision);
        }
    }

    fn answer(&mut self, index: usize, body: &str) -> Reply {
        if self.pending[index].request.kind == RequestKind::Question {
            return self.answer_question(index, body);
        }
        let Some((choice, note)) = parse_choice(body) else {
            return reply(ReplyKind::NotAReply, String::new());
        };
        let state = self.pending.remove(index);
        let rid = state.request.request_id.clone();
        let decision = match choice {
            Choice::Deny => {
                let message = if note.is_empty() {
                    DENY_MESSAGE.to_owned()
                } else {
                    format!(
                        "{}: {}",
                        DENY_MESSAGE.trim_end_matches('.'),
                        note.chars().take(NOTE_MAX_CHARS).collect::<String>()
                    )
                };
                self.finish(
                    state,
                    Behavior::Deny,
                    Reason::Denied,
                    &message,
                    None,
                    format!("Denied #{rid}."),
                )
            }
            Choice::Session if state.request.tool_name != "ExitPlanMode" => {
                let key = call_key(
                    &state.request.context,
                    &state.request.tool_name,
                    &state.request.tool_input,
                );
                self.remembered.insert(key);
                let notice = format!("Allowed #{rid}, and the same call again in this session.");
                self.finish(
                    state,
                    Behavior::Allow,
                    Reason::ApprovedSession,
                    "",
                    None,
                    notice,
                )
            }
            Choice::Allow | Choice::Session => self.finish(
                state,
                Behavior::Allow,
                Reason::Approved,
                "",
                None,
                format!("Allowed #{rid}."),
            ),
        };
        reply_decided(decision)
    }

    fn answer_question(&mut self, index: usize, body: &str) -> Reply {
        let state = &mut self.pending[index];
        let question = &state.questions[state.index];
        let text = strip(body);
        let answer = match parse_numbers(text) {
            Some(picks) => {
                let count = question.options.len();
                if picks.iter().any(|pick| *pick < 1 || *pick > count)
                    || (picks.len() > 1 && !question.multi_select)
                {
                    let several = if question.multi_select {
                        " (several allowed, separated by commas)"
                    } else {
                        ""
                    };
                    return reply(
                        ReplyKind::Invalid,
                        format!(
                            "Choose a number from 1 to {count}{several}, or type your own answer."
                        ),
                    );
                }
                picks
                    .iter()
                    .map(|pick| question.options[pick - 1].label.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            }
            None => text.to_owned(),
        };
        state
            .answers
            .insert(question.text.clone(), Value::String(answer));
        state.index += 1;
        if state.index < state.questions.len() {
            return reply(ReplyKind::Next, render_question(state));
        }
        let mut updated = state.request.tool_input.clone();
        updated.insert("answers".to_owned(), Value::Object(state.answers.clone()));
        let state = self.pending.remove(index);
        let notice = format!("Answered #{}.", state.request.request_id);
        reply_decided(self.finish(
            state,
            Behavior::Allow,
            Reason::Answered,
            "",
            Some(updated),
            notice,
        ))
    }
}

fn reply(kind: ReplyKind, notice: String) -> Reply {
    Reply {
        kind,
        notice,
        decision: None,
    }
}

fn reply_decided(decision: Decision) -> Reply {
    Reply {
        kind: ReplyKind::Decided,
        notice: String::new(),
        decision: Some(decision),
    }
}

/// The input as an object; anything else is an empty one.
fn object_of(value: &Value) -> Map<String, Value> {
    match value {
        Value::Object(members) => members.clone(),
        _ => Map::new(),
    }
}

/// Identifies "this exact call": the same tool and the same input, whatever the order of the keys (Python's `sort_keys=True`).
fn call_key(
    context: &str,
    tool_name: &str,
    tool_input: &Map<String, Value>,
) -> (String, String, String) {
    (
        context.to_owned(),
        tool_name.to_owned(),
        sorted(&Value::Object(tool_input.clone())).to_string(),
    )
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(members) => {
            let mut keys: Vec<&String> = members.keys().collect();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), sorted(&members[key])))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// The bare JID, lowercase: no resource, no surrounding whitespace.
fn bare(jid: &str) -> String {
    lower(strip(jid.split('/').next().unwrap_or("")))
}

/// The questions of an `AskUserQuestion` input, or `None` when any part is missing or of the wrong type.
fn parse_questions(tool_input: &Map<String, Value>) -> Option<Vec<Question>> {
    let Some(Value::Array(items)) = tool_input.get("questions") else {
        return None;
    };
    if items.is_empty() {
        return None;
    }
    items.iter().map(parse_question).collect()
}

fn parse_question(value: &Value) -> Option<Question> {
    let members = value.as_object()?;
    let text = members.get("question")?.as_str()?;
    let options = match members.get("options")? {
        Value::Array(options) if !options.is_empty() => options
            .iter()
            .map(parse_option)
            .collect::<Option<Vec<_>>>()?,
        _ => return None,
    };
    Some(Question {
        text: text.to_owned(),
        options,
        multi_select: truthy(members.get("multiSelect").unwrap_or(&Value::Null)),
    })
}

fn parse_option(value: &Value) -> Option<AnswerOption> {
    let members = value.as_object()?;
    let label = members.get("label")?.as_str()?;
    let description = members
        .get("description")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty());
    Some(AnswerOption {
        label: label.to_owned(),
        description: description.map(str::to_owned),
    })
}

fn render_question(state: &State) -> String {
    let question = &state.questions[state.index];
    let mut lines = vec![format!(
        "Question #{} ({} of {}): {}",
        state.request.request_id,
        state.index + 1,
        state.questions.len(),
        redact(&question.text)
    )];
    for (number, option) in question.options.iter().enumerate() {
        let description = option
            .description
            .as_ref()
            .map(|text| format!(" - {}", redact(text)))
            .unwrap_or_default();
        lines.push(format!(
            "{}. {}{description}",
            number + 1,
            redact(&option.label)
        ));
    }
    let several = if question.multi_select {
        " or several separated by commas"
    } else {
        ""
    };
    lines.push(format!(
        "Reply with a number{several}, or type your own answer."
    ));
    lines.join("\n")
}

/// A tool request takes a decision only; a question takes any text.
fn looks_like_answer(state: &State, body: &str) -> bool {
    match state.request.kind {
        RequestKind::Tool => parse_choice(body).is_some(),
        RequestKind::Question => !strip(body).is_empty(),
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
    let (word, rest) = split_first_word(strip(text))?;
    let first = lower(word);
    let rest = strip(rest);
    match first.trim_end_matches(['.', '!']) {
        "1" | "y" | "yes" | "allow" | "ok" if rest.is_empty() => Some((Choice::Allow, "")),
        "2" if rest.is_empty() => Some((Choice::Session, "")),
        "3" | "n" | "no" | "deny" | "4" => Some((Choice::Deny, rest)),
        _ => None,
    }
}

/// The `#id` of a reply and the rest of the text.
///
/// The id leads (`#a1 3`), trails (`3 #a1`) or stands alone (`#a1`); it is ASCII letters and digits.
fn split_id(text: &str) -> (Option<&str>, &str) {
    let text = strip(text);
    if let Some(after_hash) = text.strip_prefix('#') {
        let id_end = after_hash
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(after_hash.len());
        if id_end > 0 {
            let (id, rest) = after_hash.split_at(id_end);
            if rest.is_empty() {
                return (Some(id), "");
            }
            if rest.starts_with(is_space) {
                return (Some(id), strip(rest));
            }
        }
    }
    if let Some(hash) = text.rfind('#') {
        let (before, id) = (&text[..hash], &text[hash + 1..]);
        if !id.is_empty()
            && id.chars().all(|c| c.is_ascii_alphanumeric())
            && before.ends_with(is_space)
        {
            return (Some(id), strip(before));
        }
    }
    (None, text)
}

#[cfg(test)]
mod tests;
