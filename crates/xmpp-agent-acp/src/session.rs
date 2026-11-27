//! The process part: spawn the agent, run the JSON-RPC exchange, deliver events.

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::protocol::{AgentEvent, Error};

#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
}

/// A handle to one agent session; cheap to clone.
#[derive(Clone)]
pub struct AcpSession {
    session_id: String,
}

type Spawned = (AcpSession, mpsc::Receiver<AgentEvent>);

impl AcpSession {
    pub async fn spawn(_cfg: SpawnConfig) -> Result<Spawned, Error> {
        Err(Error::Exited)
    }
    pub async fn load(_cfg: SpawnConfig, _session_id: &str) -> Result<Spawned, Error> {
        Err(Error::Exited)
    }
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub async fn prompt(&self, _text: &str) -> Result<(), Error> {
        Err(Error::Exited)
    }
    pub async fn cancel(&self) -> Result<(), Error> {
        Err(Error::Exited)
    }
    pub async fn respond_permission(
        &self,
        _request_id: u64,
        _option_id: Option<&str>,
    ) -> Result<(), Error> {
        Err(Error::Exited)
    }
    pub async fn shutdown(&self) {}
}
