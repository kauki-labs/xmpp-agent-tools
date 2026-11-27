//! A fake chat, a memory log and a rig that runs the session loop against the real ACP session and the scripted fake agent.

// Each integration test file uses a different part of this module.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, sleep};
use xmpp_agent::chat::{Chat, ChatError};
use xmpp_agent::config::{Secret, Settings};
use xmpp_agent::log::{Log, LogEvent};
use xmpp_agent::session::{Session, SessionError};
use xmpp_agent_xmpp::Event;

pub const AGENT: &str = "agent@chat.example.org";
pub const ALICE: &str = "alice@chat.example.org";
pub const BOB: &str = "bob@chat.example.org";
pub const CAROL: &str = "carol@chat.example.org";

/// The longest a test waits for something that must happen.
const WAIT: Duration = Duration::from_secs(20);
/// How long a test watches for something that must not happen.
pub const QUIET: Duration = Duration::from_millis(600);

/// One thing the loop sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Out {
    Message { to: String, body: String },
    State { to: String, state: String },
}

/// A chat that records what the loop sends.
#[derive(Debug, Clone, Default)]
pub struct FakeChat {
    sent: Arc<Mutex<Vec<Out>>>,
}

impl Chat for FakeChat {
    async fn send(&self, to: &str, body: &str) -> Result<(), ChatError> {
        self.sent.lock().expect("lock").push(Out::Message {
            to: to.to_owned(),
            body: body.to_owned(),
        });
        Ok(())
    }

    async fn state(&self, to: &str, state: &str) -> Result<(), ChatError> {
        self.sent.lock().expect("lock").push(Out::State {
            to: to.to_owned(),
            state: state.to_owned(),
        });
        Ok(())
    }
}

impl FakeChat {
    pub fn all(&self) -> Vec<Out> {
        self.sent.lock().expect("lock").clone()
    }

    /// The bodies of the messages to `to`, oldest first.
    pub fn bodies(&self, to: &str) -> Vec<String> {
        self.all()
            .into_iter()
            .filter_map(|o| match o {
                Out::Message { to: t, body } if t == to => Some(body),
                _ => None,
            })
            .collect()
    }

    /// The chat states sent to `to`, oldest first.
    pub fn states(&self, to: &str) -> Vec<String> {
        self.all()
            .into_iter()
            .filter_map(|o| match o {
                Out::State { to: t, state } if t == to => Some(state),
                _ => None,
            })
            .collect()
    }

    /// Wait for a message to `to` that contains `needle`, and return its body.
    pub async fn wait_for(&self, to: &str, needle: &str) -> String {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(body) = self.bodies(to).into_iter().find(|b| b.contains(needle)) {
                return body;
            }
            assert!(
                Instant::now() < deadline,
                "no message to {to} containing {needle:?}; sent: {:#?}",
                self.all()
            );
            sleep(Duration::from_millis(20)).await;
        }
    }

    /// Wait for a chat state sent to `to`.
    pub async fn wait_for_state(&self, to: &str, state: &str) {
        let deadline = Instant::now() + WAIT;
        while !self.states(to).iter().any(|s| s == state) {
            assert!(
                Instant::now() < deadline,
                "no state {state:?} to {to}; sent: {:#?}",
                self.all()
            );
            sleep(Duration::from_millis(20)).await;
        }
    }

    /// Assert that nothing more is sent for a short while.
    pub async fn assert_quiet(&self) {
        let before = self.all().len();
        sleep(QUIET).await;
        let after = self.all();
        assert_eq!(
            after.len(),
            before,
            "unexpected output: {:#?}",
            &after[before..]
        );
    }
}

/// A log that keeps its events.
#[derive(Debug, Clone, Default)]
pub struct MemoryLog {
    events: Arc<Mutex<Vec<LogEvent>>>,
}

impl Log for MemoryLog {
    fn record(&self, event: &LogEvent) {
        self.events.lock().expect("lock").push(event.clone());
    }
}

impl MemoryLog {
    pub fn events(&self) -> Vec<LogEvent> {
        self.events.lock().expect("lock").clone()
    }
}

/// Settings for the fake agent: Alice may talk and approve, nobody else.
pub fn settings() -> Settings {
    Settings {
        jid: AGENT.to_owned(),
        password: Secret::new("pw-for-tests".to_owned()),
        server: None,
        agent_command: env!("CARGO_BIN_EXE_loop-fake-agent").to_owned(),
        agent_args: Vec::new(),
        entrypoint: "sandy".to_owned(),
        cwd: std::env::temp_dir(),
        allowed: vec![ALICE.to_owned()],
        approvers: vec![ALICE.to_owned()],
        approval_timeout: Duration::from_secs(600),
        agent_start_timeout: Duration::from_secs(60),
        log_path: None::<PathBuf>,
    }
}

/// The loop, running in a task, with a way to feed it chat events.
pub struct Rig {
    pub chat: FakeChat,
    pub log: MemoryLog,
    tx: mpsc::Sender<Event>,
    task: JoinHandle<Result<(), SessionError>>,
}

impl Rig {
    pub async fn start(settings: Settings) -> Self {
        let chat = FakeChat::default();
        let log = MemoryLog::default();
        let mut session = Session::new(settings, chat.clone(), log.clone());
        session.start().await.expect("the agent starts");
        let (tx, mut rx) = mpsc::channel(64);
        let task = tokio::spawn(async move {
            let outcome = session.run(&mut rx).await;
            session.shutdown().await;
            outcome
        });
        Self {
            chat,
            log,
            tx,
            task,
        }
    }

    /// A one-to-one message from `from` (a full or bare JID).
    pub async fn say(&self, from: &str, body: &str) {
        self.event(Event::Chat {
            from: from.to_owned(),
            id: None,
            body: body.to_owned(),
            replaces: None,
        })
        .await;
    }

    pub async fn event(&self, event: Event) {
        self.tx.send(event).await.expect("the loop reads events");
    }

    /// Close the event channel and return how the loop ended.
    pub async fn finish(self) -> Result<(), SessionError> {
        drop(self.tx);
        self.task.await.expect("the loop task")
    }

    /// Wait for the loop to end on its own (a `Disconnected` event).
    pub async fn ended(self) -> Result<(), SessionError> {
        tokio::time::timeout(WAIT, self.task)
            .await
            .expect("the loop ends")
            .expect("the loop task")
    }
}
