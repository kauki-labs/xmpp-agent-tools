//! The session loop: one human conversation with one agent.

use tokio::sync::mpsc;
use xmpp_agent_acp::SpawnConfig;
use xmpp_agent_xmpp::Event;

use crate::chat::Chat;
use crate::config::Settings;
use crate::log::Log;

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

/// The loop and its state.
pub struct Session<C, L> {
    chat: C,
    log: L,
    settings: Settings,
    spawn: SpawnConfig,
}

impl<C: Chat, L: Log> Session<C, L> {
    /// A loop for these settings. Nothing runs until [`Session::start`].
    pub fn new(settings: Settings, chat: C, log: L) -> Self {
        let spawn = settings.spawn_config();
        Self {
            chat,
            log,
            settings,
            spawn,
        }
    }

    /// Start the agent.
    pub async fn start(&mut self) -> Result<(), SessionError> {
        let _ = (&self.chat, &self.log, &self.settings, &self.spawn);
        Ok(())
    }

    /// Serve chat events until the channel closes (`Ok`) or the connection ends for good (`Err`).
    pub async fn run(&mut self, events: &mut mpsc::Receiver<Event>) -> Result<(), SessionError> {
        let _ = events;
        Ok(())
    }

    /// Stop the agent and deny what waits.
    pub async fn shutdown(&mut self) {}
}
