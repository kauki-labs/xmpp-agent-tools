//! End to end: the real `xmpp-agent` binary against a real Prosody, with the scripted fake ACP agent, driven by a second XMPP client
//! that plays the human. Skipped unless `XAT_TEST_PROSODY_BIN` points at a `prosody` binary (its `prosodyctl` sits next to it).
//!
//! One test runs every step in order: the steps share one server and one running binary, and `SSL_CERT_FILE` is process-wide. The
//! server harness follows `crates/xmpp-agent-xmpp/tests/prosody.rs`.

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;
use xmpp_agent_xmpp::{Config, Event, XmppClient};

const AGENT_BIN: &str = env!("CARGO_BIN_EXE_xmpp-agent");
const FAKE_AGENT: &str = env!("CARGO_BIN_EXE_loop-fake-agent");
const AGENT_JID: &str = "agent@localhost";

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

fn run(cmd: &mut Command) {
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    assert!(
        out.status.success(),
        "{cmd:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A throwaway CA and a leaf for `localhost`, made with `openssl`. LibreSSL (macOS) and OpenSSL both take these options.
fn make_pki(dir: &Path) {
    fs::write(dir.join("ca.cnf"), "[req]\ndistinguished_name=dn\n[dn]\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign\n").expect("write");
    fs::write(
        dir.join("leaf.cnf"),
        "basicConstraints=CA:FALSE\nsubjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\nkeyUsage=digitalSignature,keyEncipherment\n",
    )
    .expect("write");
    let ssl = |args: &[&str]| run(Command::new("openssl").args(args).current_dir(dir));
    ssl(&["genrsa", "-out", "ca.key", "2048"]);
    ssl(&[
        "req",
        "-x509",
        "-new",
        "-key",
        "ca.key",
        "-subj",
        "/CN=xat-test-ca",
        "-days",
        "2",
        "-config",
        "ca.cnf",
        "-extensions",
        "ca",
        "-out",
        "ca.pem",
    ]);
    ssl(&["genrsa", "-out", "srv.key", "2048"]);
    ssl(&[
        "req",
        "-new",
        "-key",
        "srv.key",
        "-subj",
        "/CN=localhost",
        "-out",
        "srv.csr",
    ]);
    ssl(&[
        "x509",
        "-req",
        "-in",
        "srv.csr",
        "-CA",
        "ca.pem",
        "-CAkey",
        "ca.key",
        "-CAcreateserial",
        "-days",
        "2",
        "-extfile",
        "leaf.cnf",
        "-out",
        "srv.pem",
    ]);
}

fn prosody_config(dir: &Path, c2s: u16) -> String {
    let d = dir.display();
    format!(
        r#"pidfile = "{d}/prosody.pid"
data_path = "{d}/data"
interfaces = {{ "127.0.0.1" }}
admins = {{ }}
modules_enabled = {{ "disco"; "roster"; "saslauth"; "tls"; "ping"; "carbons"; "smacks"; "csi_simple" }}
modules_disabled = {{ "s2s"; "posix" }}
c2s_ports = {{ }}
c2s_direct_tls_ports = {{ {c2s} }}
c2s_require_encryption = true
allow_unencrypted_plain_auth = false
authentication = "internal_hashed"
ssl = {{ key = "{d}/srv.key"; certificate = "{d}/srv.pem" }}
log = {{ info = "{d}/prosody.log" }}
VirtualHost "localhost"
"#
    )
}

struct Server {
    bin: PathBuf,
    dir: PathBuf,
    child: Option<Child>,
    c2s: u16,
}

impl Server {
    fn new(bin: PathBuf) -> Self {
        let dir = std::env::temp_dir().join(format!("xat-agent-prosody-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("data")).expect("dir");
        make_pki(&dir);
        let c2s = free_port();
        fs::write(dir.join("prosody.cfg.lua"), prosody_config(&dir, c2s)).expect("write");
        Self {
            bin,
            dir,
            child: None,
            c2s,
        }
    }

    fn ctl(&self, args: &[&str]) {
        let ctl = self.bin.parent().expect("bin dir").join("prosodyctl");
        run(Command::new(ctl)
            .args(args)
            .env("PROSODY_CONFIG", self.dir.join("prosody.cfg.lua")));
    }

    fn start(&mut self) {
        let child = Command::new(&self.bin)
            .arg("-F")
            .env("PROSODY_CONFIG", self.dir.join("prosody.cfg.lua"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start prosody");
        self.child = Some(child);
        let deadline = Instant::now() + Duration::from_secs(30);
        while TcpStream::connect(("127.0.0.1", self.c2s)).is_err() {
            assert!(Instant::now() < deadline, "prosody did not listen in time");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
        if std::thread::panicking() {
            let log = fs::read_to_string(self.dir.join("prosody.log")).unwrap_or_default();
            let tail: Vec<&str> = log.lines().rev().take(30).collect();
            eprintln!(
                "--- prosody log tail ---\n{}",
                tail.into_iter().rev().collect::<Vec<_>>().join("\n")
            );
            let agent_err = fs::read_to_string(self.dir.join("agent.err")).unwrap_or_default();
            eprintln!("--- xmpp-agent stderr ---\n{agent_err}");
            let ops = fs::read_to_string(self.dir.join("ops.jsonl")).unwrap_or_default();
            eprintln!("--- xmpp-agent log ---\n{ops}");
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The real binary, running. Dropping it kills it.
struct Agent {
    child: Child,
}

impl Agent {
    fn start(server: &Server) -> Self {
        let dir = &server.dir;
        fs::write(
            dir.join("agent.toml"),
            format!(
                "jid = \"{AGENT_JID}\"\nserver_host = \"localhost\"\nserver_port = {}\nagent_command = \"{FAKE_AGENT}\"\ncwd = \"{}\"\nallowed = [\"alice@localhost\"]\napproval_timeout_secs = 60\nlog_path = \"{}\"\n",
                server.c2s,
                std::env::temp_dir().display(),
                dir.join("ops.jsonl").display(),
            ),
        )
        .expect("write");
        let stderr = fs::File::create(dir.join("agent.err")).expect("file");
        let child = Command::new(AGENT_BIN)
            .args(["--config", dir.join("agent.toml").to_str().expect("utf8")])
            .env_clear()
            // The loop starts the agent through `env`, which is found on PATH.
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("SSL_CERT_FILE", dir.join("ca.pem"))
            .env("XAT_PASSWORD", "pw-agent")
            .stdout(Stdio::null())
            .stderr(stderr)
            .spawn()
            .expect("start xmpp-agent");
        Self { child }
    }

    /// Send SIGTERM and wait for the exit status.
    fn terminate(&mut self) -> std::process::ExitStatus {
        run(Command::new("kill")
            .arg("-TERM")
            .arg(self.child.id().to_string()));
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().expect("wait") {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "xmpp-agent did not stop after SIGTERM"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn connect(c2s: u16, user: &str) -> (XmppClient, Receiver<Event>) {
    let cfg = Config {
        jid: format!("{user}@localhost"),
        password: format!("pw-{user}"),
        server: Some(("localhost".into(), c2s)),
    };
    XmppClient::connect(cfg)
        .await
        .unwrap_or_else(|e| panic!("connect {user}: {e}"))
}

/// Read events until `f` accepts one. Panics with `what` on a timeout, so a failing step names itself.
async fn expect<T>(
    rx: &mut Receiver<Event>,
    secs: u64,
    what: &str,
    mut f: impl FnMut(&Event) -> Option<T>,
) -> T {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        match timeout(left, rx.recv()).await {
            Ok(Some(event)) => {
                if let Some(value) = f(&event) {
                    return value;
                }
            }
            Ok(None) => panic!("{what}: the event channel closed"),
            Err(_) => panic!("{what}: timed out after {secs} s"),
        }
    }
}

fn from_agent(event: &Event) -> Option<&str> {
    match event {
        Event::Chat {
            from,
            body,
            replaces: None,
            ..
        } if from.starts_with("agent@localhost/") => Some(body.as_str()),
        _ => None,
    }
}

/// Every message of the agent up to and including the first one that satisfies `stop`.
async fn messages_until(
    rx: &mut Receiver<Event>,
    secs: u64,
    what: &str,
    stop: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut seen = Vec::new();
    expect(rx, secs, what, |event| {
        let body = from_agent(event)?;
        seen.push(body.to_owned());
        stop(body).then_some(())
    })
    .await;
    seen
}

/// Messages the agent sends within `secs` seconds (none is the point of the call).
async fn agent_messages_within(rx: &mut Receiver<Event>, secs: u64) -> Vec<String> {
    let mut seen = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while let Ok(Some(event)) = timeout(
        deadline.saturating_duration_since(tokio::time::Instant::now()),
        rx.recv(),
    )
    .await
    {
        if let Some(body) = from_agent(&event) {
            seen.push(body.to_owned());
        }
    }
    seen
}

/// Ask `/status` until the binary answers: it needs a moment after a start and after a server restart.
async fn wait_until_ready(human: &XmppClient, rx: &mut Receiver<Event>, what: &str) {
    for _ in 0..30 {
        let _ = human.send_chat(AGENT_JID, "/status").await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while let Ok(Some(event)) = timeout(
            deadline.saturating_duration_since(tokio::time::Instant::now()),
            rx.recv(),
        )
        .await
        {
            if from_agent(&event).is_some_and(|body| body.contains("Agent: running")) {
                return;
            }
        }
    }
    panic!("{what}: the agent did not answer /status in time");
}

fn pass(name: &str) {
    eprintln!("PASS {name}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn xmpp_agent_against_a_real_prosody() {
    let Some(bin) = std::env::var_os("XAT_TEST_PROSODY_BIN") else {
        eprintln!("skipped: XAT_TEST_PROSODY_BIN is not set");
        return;
    };
    let mut server = Server::new(PathBuf::from(bin));
    for user in ["agent", "alice", "bob"] {
        server.ctl(&["register", user, "localhost", &format!("pw-{user}")]);
    }
    server.start();
    // The throwaway CA is the only trust root this process needs; no system store is touched.
    unsafe { std::env::set_var("SSL_CERT_FILE", server.dir.join("ca.pem")) };
    let mut agent = Agent::start(&server);
    let (alice, mut alice_rx) = connect(server.c2s, "alice").await;
    let (bob, mut bob_rx) = connect(server.c2s, "bob").await;
    wait_until_ready(&alice, &mut alice_rx, "start").await;
    pass("the binary starts, logs in over direct TLS and answers /status");

    // A prompt gets an answer as a direct message, and one redacted line for the tool call.
    alice.send_chat(AGENT_JID, "hello").await.expect("send");
    let seen = messages_until(&mut alice_rx, 20, "prompt answer", |b| b == "hello").await;
    assert!(
        seen.iter()
            .any(|b| b.starts_with("tool: ") && b.contains("ls")),
        "no tool line in {seen:?}"
    );
    pass("a prompt gets its answer, and a tool line");

    // A permission asked in chat and approved with 1: the agent proceeds.
    alice.send_chat(AGENT_JID, "ask").await.expect("send");
    let seen = messages_until(&mut alice_rx, 20, "approval prompt", |b| {
        b.contains("Approval #")
    })
    .await;
    assert!(
        seen.last().is_some_and(|b| b.contains("touch a.txt")),
        "{seen:?}"
    );
    alice.send_chat(AGENT_JID, "1").await.expect("send");
    messages_until(&mut alice_rx, 20, "agent proceeds after 1", |b| {
        b == "allowed:allow"
    })
    .await;
    pass("a permission is asked in chat and approved with 1");

    // A permission denied with 3.
    alice.send_chat(AGENT_JID, "ask").await.expect("send");
    messages_until(&mut alice_rx, 20, "second approval prompt", |b| {
        b.contains("Approval #")
    })
    .await;
    alice.send_chat(AGENT_JID, "3").await.expect("send");
    messages_until(&mut alice_rx, 20, "agent is refused after 3", |b| {
        b == "rejected:reject"
    })
    .await;
    pass("a permission is denied with 3");

    // A stranger gets no reply, and the agent says nothing to the human either.
    bob.send_chat(AGENT_JID, "hello").await.expect("send");
    bob.send_chat(AGENT_JID, "/new").await.expect("send");
    let to_bob = agent_messages_within(&mut bob_rx, 3).await;
    assert!(to_bob.is_empty(), "the stranger got {to_bob:?}");
    let to_alice = agent_messages_within(&mut alice_rx, 1).await;
    assert!(
        to_alice.is_empty(),
        "the stranger's messages reached the human: {to_alice:?}"
    );
    pass("a stranger gets no reply");

    // /stop cancels a running turn.
    alice.send_chat(AGENT_JID, "slow").await.expect("send");
    expect(&mut alice_rx, 20, "typing indication", |e| matches!(e, Event::ChatState { from, state } if from.starts_with("agent@localhost/") && state == "composing").then_some(())).await;
    alice.send_chat(AGENT_JID, "/stop").await.expect("send");
    messages_until(&mut alice_rx, 20, "stopped", |b| b == "Stopped.").await;
    pass("/stop cancels the turn mid-turn");

    // A server restart with stream management on: the binary reconnects and a new prompt still works.
    server.stop();
    expect(&mut alice_rx, 30, "alice sees the drop", |e| {
        matches!(e, Event::Reconnecting).then_some(())
    })
    .await;
    server.start();
    expect(&mut alice_rx, 60, "alice reconnects", |e| {
        matches!(e, Event::Reconnected).then_some(())
    })
    .await;
    wait_until_ready(&alice, &mut alice_rx, "after the restart").await;
    alice.send_chat(AGENT_JID, "hello").await.expect("send");
    messages_until(&mut alice_rx, 30, "answer after the restart", |b| {
        b == "hello"
    })
    .await;
    pass("after a Prosody restart a new prompt still works");

    // SIGTERM stops the binary cleanly, and the operational log tells what happened and holds no message body.
    let status = agent.terminate();
    assert!(status.success(), "exit status {status}");
    let ops = fs::read_to_string(server.dir.join("ops.jsonl")).expect("ops log");
    for needle in [
        r#""event":"started""#,
        r#""event":"approval_asked""#,
        r#""event":"approval_decided""#,
        r#""event":"ignored_sender""#,
        r#""event":"stopped""#,
    ] {
        assert!(ops.contains(needle), "{needle} is not in the log:\n{ops}");
    }
    assert!(
        ops.contains(r#""behavior":"allow""#) && ops.contains(r#""behavior":"deny""#),
        "{ops}"
    );
    assert!(ops.contains("bob@localhost"), "{ops}");
    assert!(
        !ops.contains("pw-agent")
            && !ops.contains("allowed:allow")
            && !ops.contains("rejected:reject"),
        "{ops}"
    );
    pass("SIGTERM stops the binary with exit 0 and the operational log is complete");
}
