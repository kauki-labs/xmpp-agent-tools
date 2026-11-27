//! The process part: spawn the agent, run the JSON-RPC exchange, deliver events.
//!
//! One reader task decodes the agent's stdout and is the only producer of events, so `Done` always follows the turn's updates.
//! Dropping every handle (or calling [`AcpSession::shutdown`]) kills the agent.

use std::collections::{HashMap, VecDeque};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

use crate::protocol::{self, AgentEvent, Error, Incoming};

const EVENT_BUFFER: usize = 1024;
const STDERR_LINES: usize = 20;

#[derive(Debug, Clone)]
pub struct SpawnConfig {
    pub command: String,
    pub args: Vec<String>,
    /// Added to the inherited environment.
    pub env: Vec<(String, String)>,
    pub cwd: PathBuf,
}

type RpcResult = Result<Value, (i64, String)>;

enum Pending {
    Call(oneshot::Sender<RpcResult>),
    /// A `session/prompt`: its response becomes [`AgentEvent::Done`].
    Prompt,
}

struct Shared {
    stdin: tokio::sync::Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Pending>>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    /// Set while `session/load` replays history: those updates are dropped, not posted again.
    replaying: AtomicBool,
}

impl Shared {
    async fn write(&self, msg: &Value) -> Result<(), Error> {
        let mut line = msg.to_string();
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        let r = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        }
        .await;
        r.map_err(|e| match e.kind() {
            ErrorKind::BrokenPipe => Error::Exited,
            _ => Error::Io(e),
        })
    }

    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn register(&self, id: u64, p: Pending) {
        self.pending.lock().unwrap().insert(id, p);
    }

    fn unregister(&self, id: u64) {
        self.pending.lock().unwrap().remove(&id);
    }

    async fn call(&self, build: impl FnOnce(u64) -> Value) -> Result<Value, Error> {
        let id = self.id();
        let (tx, rx) = oneshot::channel();
        self.register(id, Pending::Call(tx));
        if let Err(e) = self.write(&build(id)).await {
            self.unregister(id);
            return Err(e);
        }
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err((code, message))) => Err(Error::Rpc { code, message }),
            Err(_) => Err(Error::Exited),
        }
    }
}

/// A handle to one agent session; cheap to clone.
#[derive(Clone)]
pub struct AcpSession {
    session_id: String,
    shared: Arc<Shared>,
}

type Spawned = (AcpSession, mpsc::Receiver<AgentEvent>);

impl AcpSession {
    /// Start the agent, run `initialize` and `session/new`.
    pub async fn spawn(cfg: SpawnConfig) -> Result<Spawned, Error> {
        let (shared, rx) = start(&cfg)?;
        shared.call(protocol::initialize_request).await?;
        let r = shared
            .call(|id| protocol::new_session_request(id, &cfg.cwd))
            .await?;
        let session_id = r
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("session/new result has no sessionId".into()))?
            .to_string();
        Ok((AcpSession { session_id, shared }, rx))
    }

    /// Start the agent and `session/load` an existing session. The history the agent replays is dropped.
    pub async fn load(cfg: SpawnConfig, session_id: &str) -> Result<Spawned, Error> {
        let (shared, rx) = start(&cfg)?;
        shared.call(protocol::initialize_request).await?;
        shared.replaying.store(true, Ordering::SeqCst);
        shared
            .call(|id| protocol::load_session_request(id, session_id, &cfg.cwd))
            .await?;
        Ok((
            AcpSession {
                session_id: session_id.to_string(),
                shared,
            },
            rx,
        ))
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Send a prompt. It returns once the prompt is written; the turn's events follow on the channel and end with `Done`.
    /// A prompt sent mid-turn is queued by the agent.
    pub async fn prompt(&self, text: &str) -> Result<(), Error> {
        let id = self.shared.id();
        self.shared.register(id, Pending::Prompt);
        let msg = protocol::prompt_request(id, &self.session_id, text);
        if let Err(e) = self.shared.write(&msg).await {
            self.shared.unregister(id);
            return Err(e);
        }
        Ok(())
    }

    pub async fn cancel(&self) -> Result<(), Error> {
        self.shared
            .write(&protocol::cancel_notification(&self.session_id))
            .await
    }

    /// Answer an [`AgentEvent::Permission`]: the chosen option id, or `None` for cancelled.
    pub async fn respond_permission(
        &self,
        request_id: u64,
        option_id: Option<&str>,
    ) -> Result<(), Error> {
        self.shared
            .write(&protocol::permission_response(request_id, option_id))
            .await
    }

    /// Kill the agent. The channel then yields `Exited`.
    pub async fn shutdown(&self) {
        if let Some(kill) = self.shared.kill.lock().unwrap().take() {
            let _ = kill.send(());
        }
    }
}

fn start(cfg: &SpawnConfig) -> Result<(Arc<Shared>, mpsc::Receiver<AgentEvent>), Error> {
    let mut child = Command::new(&cfg.command)
        .args(&cfg.args)
        .envs(cfg.env.iter().map(|(k, v)| (k, v)))
        .current_dir(&cfg.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child.stdin.take().expect("stdin is piped");
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");

    let (kill_tx, kill_rx) = oneshot::channel::<()>();
    let (exit_tx, exit_rx) = oneshot::channel::<String>();
    tokio::spawn(async move {
        let why = tokio::select! {
            status = child.wait() => match status {
                Ok(s) => format!("the agent exited: {s}"),
                Err(e) => format!("waiting for the agent failed: {e}"),
            },
            // Also fires when every handle is dropped.
            _ = kill_rx => {
                let _ = child.kill().await;
                "the agent was stopped".to_string()
            }
        };
        let _ = exit_tx.send(why);
    });

    let tail = Arc::new(Mutex::new(VecDeque::<String>::new()));
    let tail_w = tail.clone();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            let mut t = tail_w.lock().unwrap();
            if t.len() == STDERR_LINES {
                t.pop_front();
            }
            t.push_back(l);
        }
    });

    let shared = Arc::new(Shared {
        stdin: tokio::sync::Mutex::new(stdin),
        next_id: AtomicU64::new(1),
        pending: Mutex::new(HashMap::new()),
        kill: Mutex::new(Some(kill_tx)),
        replaying: AtomicBool::new(false),
    });
    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    tokio::spawn(read_loop(
        Arc::downgrade(&shared),
        stdout,
        tx,
        exit_rx,
        stderr_task,
        tail,
    ));
    Ok((shared, rx))
}

async fn read_loop(
    shared: Weak<Shared>,
    stdout: tokio::process::ChildStdout,
    tx: mpsc::Sender<AgentEvent>,
    exit_rx: oneshot::Receiver<String>,
    stderr_task: tokio::task::JoinHandle<()>,
    tail: Arc<Mutex<VecDeque<String>>>,
) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Some(sh) = shared.upgrade() else { return };
        let Ok(msg) = protocol::parse_message(&line) else {
            continue;
        };
        let event = match msg {
            Incoming::Response { id, result } => {
                let pending = sh.pending.lock().unwrap().remove(&id);
                match pending {
                    Some(Pending::Call(reply)) => {
                        sh.replaying.store(false, Ordering::SeqCst);
                        let _ = reply.send(result);
                        None
                    }
                    Some(Pending::Prompt) => Some(AgentEvent::Done {
                        stop_reason: match result {
                            Ok(v) => protocol::stop_reason(&v),
                            Err((_, message)) => format!("error: {message}"),
                        },
                    }),
                    None => None,
                }
            }
            Incoming::Notification { method, params } => {
                if method == "session/update" && !sh.replaying.load(Ordering::SeqCst) {
                    protocol::decode_update(&params)
                } else {
                    None
                }
            }
            Incoming::Request { id, method, params } => {
                if method == "session/request_permission" {
                    match protocol::decode_permission(id, &params) {
                        Ok(ev) => Some(ev),
                        // Fail closed: a request this adapter cannot read is cancelled.
                        Err(_) => {
                            let _ = sh.write(&protocol::permission_response(id, None)).await;
                            None
                        }
                    }
                } else {
                    let _ = sh
                        .write(&protocol::error_response(id, -32601, "method not found"))
                        .await;
                    None
                }
            }
        };
        drop(sh);
        if let Some(ev) = event
            && tx.send(ev).await.is_err()
        {
            return;
        }
    }
    if let Some(sh) = shared.upgrade() {
        sh.pending.lock().unwrap().clear();
    }
    let why = timeout(Duration::from_secs(5), exit_rx)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| "the agent closed its output".into());
    let _ = timeout(Duration::from_secs(1), stderr_task).await;
    let stderr = tail
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let msg = if stderr.is_empty() {
        why
    } else {
        format!("{why}; stderr: {stderr}")
    };
    let _ = tx.send(AgentEvent::Exited(msg)).await;
}
