//! The session loop: one human conversation with one agent, in direct messages.
//!
//! One task owns all the state and reacts to three sources, agent events first: the agent's events, the chat's events and a timer
//! for approval timeouts. The loop fails closed: an unknown sender is ignored, an approval that nobody answers is denied, and a dead
//! agent gets no more prompts until `/new`.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, interval, timeout, timeout_at};
use xmpp_agent_acp::{AcpSession, AgentEvent, Error as AcpError, PermOption, SpawnConfig};
use xmpp_agent_approvals::{
    Behavior, Broker, DEFAULT_MAX_PENDING, DENY_MESSAGE, Decision, Reason as ApprovalReason,
    ReplyKind,
};
use xmpp_agent_client::router::{Local, Routed, route};
use xmpp_agent_common::redact::redact;
use xmpp_agent_xmpp::Event;

use crate::chat::Chat;
use crate::config::Settings;
use crate::format::{
    HELP, blocked_prompt, denied_prompt, display_tool, honest_approval_text, pick_option,
    rejection_text, status_text, tool_line, turn_messages,
};
use crate::log::{ERROR_MAX_CHARS, Log, LogEvent};

/// The approvals context: there is one conversation, so one context.
const CONTEXT: &str = "dm";

/// The longest [`Session::start`] and `/new` wait for the agent's command list.
const COMMANDS_WAIT: Duration = Duration::from_secs(1);

/// How often the loop looks for approvals that ran out of time.
const TICK: Duration = Duration::from_millis(250);

/// Commands the agent lists but that only work in a terminal (docs/experiments.md E6).
const TERMINAL_COMMANDS: [&str; 4] = ["doctor", "color", "focus", "reload-plugins"];

/// Why the loop ended with an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    /// The agent could not be started.
    Agent(String),
    /// The chat connection ended for good.
    Disconnected(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Agent(text) => write!(f, "agent: {text}"),
            Self::Disconnected(text) => write!(f, "disconnected: {text}"),
        }
    }
}

impl std::error::Error for SessionError {}

/// A permission request that waits for a human.
struct Waiting {
    /// The id the agent gave the request; the answer goes back under it.
    acp_id: u64,
    tool: String,
    options: Vec<PermOption>,
}

/// The loop and its state.
pub struct Session<C, L> {
    chat: C,
    log: L,
    settings: Settings,
    spawn: SpawnConfig,
    /// Behind a mutex only so the loop is `Send`; it is never locked across an await.
    broker: Mutex<Broker>,
    agent: Option<AcpSession>,
    agent_events: Option<mpsc::Receiver<AgentEvent>>,
    harness_commands: Vec<String>,
    /// Bare JID of the human who sent the last prompt; the agent's output goes to them.
    reply_to: Option<String>,
    turn_text: String,
    /// Prompts sent whose turn has not ended.
    turns_open: usize,
    /// Set once the human was told the agent is gone; cleared by `/new`.
    told_dead: bool,
    waiting: HashMap<String, Waiting>,
}

fn bare(jid: &str) -> String {
    jid.split('/').next().unwrap_or("").to_lowercase()
}

fn clean(text: &str) -> String {
    redact(text).chars().take(ERROR_MAX_CHARS).collect()
}

async fn next_agent_event(events: &mut Option<mpsc::Receiver<AgentEvent>>) -> Option<AgentEvent> {
    match events {
        Some(events) => events.recv().await,
        None => std::future::pending().await,
    }
}

impl<C: Chat, L: Log> Session<C, L> {
    fn broker(&self) -> MutexGuard<'_, Broker> {
        self.broker.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A loop for these settings. Nothing runs until [`Session::start`].
    pub fn new(settings: Settings, chat: C, log: L) -> Self {
        let spawn = settings.spawn_config();
        let started = Instant::now();
        let mut counter = 0u64;
        let broker = Broker::new(
            settings.approvers.clone(),
            Box::new(move || started.elapsed().as_secs_f64()),
            Box::new(move || {
                counter += 1;
                format!("a{counter}")
            }),
            settings.approval_timeout.as_secs_f64(),
            DEFAULT_MAX_PENDING,
        );
        Self {
            chat,
            log,
            settings,
            spawn,
            broker: Mutex::new(broker),
            agent: None,
            agent_events: None,
            harness_commands: Vec::new(),
            reply_to: None,
            turn_text: String::new(),
            turns_open: 0,
            told_dead: false,
            waiting: HashMap::new(),
        }
    }

    /// Start the agent.
    pub async fn start(&mut self) -> Result<(), SessionError> {
        self.log.record(&LogEvent::Started);
        self.spawn_agent()
            .await
            .map_err(|e| SessionError::Agent(clean(&e)))
    }

    /// Serve chat events until the channel closes (`Ok`) or the connection ends for good (`Err`).
    pub async fn run(&mut self, events: &mut mpsc::Receiver<Event>) -> Result<(), SessionError> {
        let mut tick = interval(TICK);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                event = next_agent_event(&mut self.agent_events) => self.on_agent(event).await,
                event = events.recv() => match event {
                    None => return Ok(()),
                    Some(Event::Disconnected(reason)) => {
                        self.log.record(&LogEvent::Error { what: format!("disconnected: {reason}") });
                        return Err(SessionError::Disconnected(reason));
                    }
                    Some(event) => self.on_chat(event).await,
                },
                _ = tick.tick() => {
                    let due = self.broker().tick();
                    for decision in due {
                        self.apply(decision).await;
                    }
                }
            }
        }
    }

    /// Stop the agent and deny what waits.
    pub async fn shutdown(&mut self) {
        self.close_approvals();
        if let Some(agent) = self.agent.take() {
            agent.shutdown().await;
        }
        self.agent_events = None;
        self.log.record(&LogEvent::Stopped);
    }

    /// Start an agent session. An agent that hangs before it opens one must not freeze the loop, so the start has a time limit.
    async fn spawn_agent(&mut self) -> Result<(), String> {
        let limit = self.settings.agent_start_timeout;
        let started = match timeout(limit, AcpSession::spawn(self.spawn.clone())).await {
            Ok(Ok(started)) => started,
            Ok(Err(error)) => return Err(error.to_string()),
            Err(_) => {
                return Err(format!(
                    "the agent did not open a session; timed out after {} s",
                    limit.as_secs()
                ));
            }
        };
        let (agent, events) = started;
        self.log.record(&LogEvent::AgentStarted {
            session_id: agent.session_id().to_owned(),
        });
        self.agent = Some(agent);
        self.agent_events = Some(events);
        self.told_dead = false;
        self.harness_commands.clear();
        self.await_commands().await;
        Ok(())
    }

    /// The agent lists its commands right after it opens a session. Wait a moment for the list, so a `/command` sent at once is routed
    /// with it. An agent that sends none costs the wait and nothing else.
    async fn await_commands(&mut self) {
        let deadline = Instant::now() + COMMANDS_WAIT;
        while self.harness_commands.is_empty() {
            let Some(events) = self.agent_events.as_mut() else {
                return;
            };
            match timeout_at(deadline, events.recv()).await {
                Ok(Some(event)) => self.on_agent(Some(event)).await,
                Ok(None) | Err(_) => return,
            }
        }
    }

    async fn say(&self, to: &str, body: &str) {
        if let Err(error) = self.chat.send(to, body).await {
            self.log.record(&LogEvent::Error {
                what: format!("send failed: {error}"),
            });
        }
    }

    async fn typing(&self, to: &str, state: &str) {
        // A lost typing indication is not worth a log line.
        let _ = self.chat.state(to, state).await;
    }

    async fn on_chat(&mut self, event: Event) {
        match event {
            // A correction (XEP-0308) is an edit; edits are out of scope.
            Event::Chat {
                replaces: Some(_), ..
            } => {}
            Event::Chat { from, body, .. } => self.on_message(&from, &body).await,
            Event::Groupchat { .. }
            | Event::Reaction { .. }
            | Event::ChatState { .. }
            | Event::Presence { .. }
            | Event::Reconnecting
            | Event::Reconnected
            | Event::Disconnected(_) => {}
        }
    }

    async fn on_message(&mut self, from: &str, body: &str) {
        let sender = bare(from);
        let may_talk = self.settings.allowed.contains(&sender);
        if !may_talk && !self.settings.approvers.contains(&sender) {
            self.log.record(&LogEvent::IgnoredSender { from: sender });
            return;
        }
        // An approver may answer approvals without being allowed to talk to the agent. The broker decides who counts.
        let reply = self.broker().handle_reply(&sender, CONTEXT, body);
        match reply.kind {
            ReplyKind::NotAReply if may_talk => {}
            ReplyKind::NotAReply => {
                self.log.record(&LogEvent::IgnoredSender { from: sender });
                return;
            }
            ReplyKind::Decided | ReplyKind::Next => {
                // The confirmation of a decision is in the decision's notice, that of a question step in the reply's.
                let notice = match &reply.decision {
                    Some(decision) if reply.notice.is_empty() => decision.notice.clone(),
                    _ => reply.notice.clone(),
                };
                if !notice.is_empty() {
                    self.say(&sender, &honest_approval_text(&notice)).await;
                }
                if let Some(decision) = reply.decision {
                    self.apply(decision).await;
                }
                return;
            }
            ReplyKind::Forbidden => {
                let notice = if reply.notice.is_empty() {
                    "Only an approver can answer an approval request.".to_owned()
                } else {
                    reply.notice
                };
                self.say(&sender, &notice).await;
                return;
            }
            ReplyKind::UnknownRequest | ReplyKind::Ambiguous | ReplyKind::Invalid => {
                if !reply.notice.is_empty() {
                    self.say(&sender, &reply.notice).await;
                }
                return;
            }
        }
        let routed = {
            let names: Vec<&str> = self.harness_commands.iter().map(String::as_str).collect();
            route(body, &names, &TERMINAL_COMMANDS)
        };
        match routed {
            Routed::Prompt(prompt) => self.prompt(&sender, &prompt.text).await,
            Routed::Harness(command) => self.prompt(&sender, &command.text).await,
            Routed::Local(command) => self.local(&sender, &command).await,
            Routed::Rejected(rejected) => {
                if let Some(text) = rejection_text(&rejected) {
                    self.say(&sender, &text).await;
                }
            }
        }
    }

    async fn local(&mut self, sender: &str, command: &Local) {
        match command.name.as_str() {
            "new" => self.new_session(sender).await,
            "stop" => match self.agent.clone() {
                Some(agent) if self.turns_open > 0 => {
                    if let Err(error) = agent.cancel().await {
                        self.log.record(&LogEvent::Error {
                            what: format!("cancel failed: {error}"),
                        });
                    }
                    // A request that waits would otherwise time out later and restart the work the human just stopped.
                    self.cancel_approvals().await;
                }
                _ => self.say(sender, "Nothing to stop.").await,
            },
            "status" => {
                let text = status_text(
                    self.agent.is_some(),
                    self.turns_open > 0,
                    self.agent.as_ref().map(AcpSession::session_id),
                    self.broker().pending(None).len(),
                );
                self.say(sender, &text).await;
            }
            "help" => self.say(sender, HELP).await,
            other => {
                self.say(
                    sender,
                    &format!(
                        "/{other} is not available in this version. Send /help for the commands."
                    ),
                )
                .await
            }
        }
    }

    /// Send a prompt to the agent. A dead agent gets none: the human was told once and must send `/new`.
    async fn prompt(&mut self, sender: &str, text: &str) {
        let Some(agent) = self.agent.clone() else {
            return;
        };
        self.reply_to = Some(sender.to_owned());
        self.turns_open += 1;
        self.typing(sender, "composing").await;
        if let Err(error) = agent.prompt(text).await {
            self.turns_open = self.turns_open.saturating_sub(1);
            match error {
                AcpError::Exited => self.agent_gone("the agent has exited".to_owned()).await,
                other => self.log.record(&LogEvent::Error {
                    what: format!("prompt failed: {other}"),
                }),
            }
        }
    }

    async fn new_session(&mut self, sender: &str) {
        self.close_approvals();
        if let Some(agent) = self.agent.take() {
            agent.shutdown().await;
        }
        self.agent_events = None;
        self.turn_text.clear();
        self.turns_open = 0;
        self.reply_to = Some(sender.to_owned());
        match self.spawn_agent().await {
            Ok(()) => self.say(sender, "New session started.").await,
            Err(error) => {
                let text = clean(&error);
                self.log.record(&LogEvent::Error {
                    what: format!("agent did not start: {text}"),
                });
                self.told_dead = true;
                self.say(
                    sender,
                    &format!("The agent did not start: {text}. Send /new to try again."),
                )
                .await;
            }
        }
    }

    /// Close every waiting approval and answer the agent "cancelled". Used by `/stop`; the agent is alive.
    async fn cancel_approvals(&mut self) {
        let closed = self.broker().close();
        for decision in closed {
            if let Some(waiting) = self.waiting.remove(&decision.request.request_id) {
                self.log_decision(&decision, &waiting.tool);
                self.answer_agent(waiting.acp_id, None).await;
            }
        }
        self.waiting.clear();
        self.broker().forget(CONTEXT);
    }

    /// Deny what waits without answering the agent: it is gone or about to be.
    fn close_approvals(&mut self) {
        let closed = self.broker().close();
        for decision in closed {
            if let Some(waiting) = self.waiting.remove(&decision.request.request_id) {
                self.log_decision(&decision, &waiting.tool);
            }
        }
        self.waiting.clear();
        self.broker().forget(CONTEXT);
    }

    fn log_decision(&self, decision: &Decision, tool: &str) {
        self.log.record(&LogEvent::ApprovalDecided {
            request_id: decision.request.request_id.clone(),
            tool: tool.to_owned(),
            behavior: decision.behavior.as_str().to_owned(),
            reason: decision.reason.as_str().to_owned(),
        });
    }

    async fn on_agent(&mut self, event: Option<AgentEvent>) {
        match event {
            None => self.agent_gone("the agent channel closed".to_owned()).await,
            Some(AgentEvent::Exited(reason)) => self.agent_gone(reason).await,
            Some(AgentEvent::Text(text)) => self.turn_text.push_str(&text),
            Some(AgentEvent::ToolCall {
                title, raw_input, ..
            }) => {
                if let Some(to) = self.reply_to.clone() {
                    self.say(&to, &tool_line(&title, &raw_input)).await;
                }
            }
            Some(AgentEvent::Commands(names)) => self.harness_commands = names,
            Some(AgentEvent::Permission {
                request_id,
                tool,
                options,
            }) => self.on_permission(request_id, tool, options).await,
            Some(AgentEvent::Done { stop_reason }) => {
                self.turns_open = self.turns_open.saturating_sub(1);
                let text = std::mem::take(&mut self.turn_text);
                if let Some(to) = self.reply_to.clone() {
                    for message in turn_messages(&text, &stop_reason) {
                        self.say(&to, &message).await;
                    }
                    // Typing goes on while a queued prompt still has a turn open.
                    if self.turns_open == 0 {
                        self.typing(&to, "active").await;
                    }
                }
            }
            Some(AgentEvent::Thought(_) | AgentEvent::ToolUpdate { .. }) => {}
        }
    }

    async fn on_permission(&mut self, acp_id: u64, tool: String, options: Vec<PermOption>) {
        let tool = display_tool(&tool);
        // The event carries no raw input, so the broker sees an empty one.
        let submitted = self
            .broker()
            .submit(CONTEXT, &tool, &Value::Null, &acp_id.to_string());
        let submitted = match submitted {
            Ok(submitted) => submitted,
            Err(error) => {
                self.log.record(&LogEvent::Error {
                    what: format!("approval refused: {error}"),
                });
                self.answer_agent(acp_id, pick_option(&options, false))
                    .await;
                return;
            }
        };
        if let Some(decision) = submitted.decision {
            self.waiting.insert(
                decision.request.request_id.clone(),
                Waiting {
                    acp_id,
                    tool,
                    options,
                },
            );
            self.apply(decision).await;
            return;
        }
        let (Some(request), Some(text)) = (submitted.request, submitted.text) else {
            self.answer_agent(acp_id, pick_option(&options, false))
                .await;
            return;
        };
        self.log.record(&LogEvent::ApprovalAsked {
            request_id: request.request_id.clone(),
            tool: tool.clone(),
        });
        self.waiting.insert(
            request.request_id,
            Waiting {
                acp_id,
                tool,
                options,
            },
        );
        let text = honest_approval_text(&text);
        for approver in self.settings.approvers.clone() {
            self.say(&approver, &text).await;
        }
    }

    async fn answer_agent(&self, acp_id: u64, option: Option<String>) {
        if let Some(agent) = &self.agent
            && let Err(error) = agent.respond_permission(acp_id, option.as_deref()).await
        {
            self.log.record(&LogEvent::Error {
                what: format!("answering the agent failed: {error}"),
            });
        }
    }

    /// Turn a broker decision into the agent's answer, and tell the agent why when it was denied for a reason.
    async fn apply(&mut self, decision: Decision) {
        let Some(waiting) = self.waiting.remove(&decision.request.request_id) else {
            return;
        };
        self.log_decision(&decision, &waiting.tool);
        let allow = decision.behavior == Behavior::Allow;
        // The event has no raw input, so "allow again in this session" cannot match an exact call: never remember it.
        if decision.reason == ApprovalReason::ApprovedSession {
            self.broker().forget(CONTEXT);
        }
        self.answer_agent(waiting.acp_id, pick_option(&waiting.options, allow))
            .await;
        if allow {
            return;
        }
        let follow_up = match decision.reason {
            ApprovalReason::Timeout => {
                for approver in self.settings.approvers.clone() {
                    self.say(&approver, &decision.notice).await;
                }
                Some(blocked_prompt(&waiting.tool))
            }
            ApprovalReason::Denied if decision.message != DENY_MESSAGE => {
                Some(denied_prompt(&waiting.tool, &decision.message))
            }
            _ => None,
        };
        if let (Some(text), Some(to)) = (follow_up, self.reply_to.clone()) {
            self.prompt(&to, &text).await;
        }
    }

    /// The agent is gone: deny what waits, tell the human once, and wait for `/new`.
    async fn agent_gone(&mut self, reason: String) {
        self.agent = None;
        self.agent_events = None;
        self.close_approvals();
        self.turn_text.clear();
        self.turns_open = 0;
        self.log.record(&LogEvent::AgentExited {
            reason: reason.clone(),
        });
        if self.told_dead {
            return;
        }
        self.told_dead = true;
        let text = format!(
            "The agent stopped ({}). Send /new to start a new session.",
            clean(&reason)
        );
        let targets = match self.reply_to.clone() {
            Some(to) => vec![to],
            None => self.settings.allowed.clone(),
        };
        for to in targets {
            self.typing(&to, "active").await;
            self.say(&to, &text).await;
        }
    }
}
