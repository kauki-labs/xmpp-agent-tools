//! End-to-end run against a throwaway Prosody. Skipped unless `XAT_TEST_PROSODY_BIN` points at a `prosody` binary (its
//! `prosodyctl` sits next to it). One test runs every check in order: they share one server, and the checks set
//! `SSL_CERT_FILE`, which is process-wide.

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;
use xmpp_agent_xmpp::{Config, Event, XmppClient};

const ROOM: &str = "room@rooms.localhost";

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
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
    let p = |n: &str| dir.join(n);
    fs::write(p("ca.cnf"), "[req]\ndistinguished_name=dn\n[dn]\n[ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign\n").unwrap();
    fs::write(p("leaf.cnf"), "basicConstraints=CA:FALSE\nsubjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\nkeyUsage=digitalSignature,keyEncipherment\n").unwrap();
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

/// One CA for every test in this process: `SSL_CERT_FILE` is process-wide, so it is set once, here. The throwaway CA is the
/// only trust root the process needs; no system store is touched.
fn shared_pki() -> &'static Path {
    static PKI: OnceLock<PathBuf> = OnceLock::new();
    PKI.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("xat-pki-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        make_pki(&dir);
        unsafe { std::env::set_var("SSL_CERT_FILE", dir.join("ca.pem")) };
        dir
    })
}

fn prosody_config(dir: &Path, c2s: u16, http: u16, smacks: bool) -> String {
    let d = dir.display();
    let smacks = if smacks { r#""smacks"; "# } else { "" };
    format!(
        r#"pidfile = "{d}/prosody.pid"
data_path = "{d}/data"
interfaces = {{ "127.0.0.1" }}
admins = {{ }}
modules_enabled = {{ "disco"; "roster"; "saslauth"; "tls"; "ping"; "mam"; "carbons"; {smacks}"csi_simple"; "http" }}
modules_disabled = {{ "s2s"; "posix" }}
c2s_ports = {{ }}
c2s_direct_tls_ports = {{ {c2s} }}
c2s_require_encryption = true
allow_unencrypted_plain_auth = false
authentication = "internal_hashed"
default_archive_policy = true
ssl = {{ key = "{d}/srv.key"; certificate = "{d}/srv.pem" }}
http_ports = {{ {http} }}
https_ports = {{ }}
http_interfaces = {{ "127.0.0.1" }}
log = {{ info = "{d}/prosody.log" }}
VirtualHost "localhost"
Component "rooms.localhost" "muc"
  modules_enabled = {{ "muc_mam" }}
  restrict_room_creation = false
  muc_room_locking = false
  muc_room_default_public_jids = true
  muc_log_all_rooms = true
Component "upload.localhost" "http_file_share"
  http_host = "localhost"
  http_external_url = "http://localhost:{http}/"
  http_file_share_size_limit = 5 * 1024 * 1024
"#
    )
}

struct Server {
    bin: PathBuf,
    dir: PathBuf,
    child: Option<Child>,
    c2s: u16,
    http: u16,
}

impl Server {
    fn new(bin: PathBuf) -> Server {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let n = COUNT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("xat-prosody-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("data")).unwrap();
        let pki = shared_pki();
        for f in ["srv.key", "srv.pem"] {
            fs::copy(pki.join(f), dir.join(f)).unwrap();
        }
        let (c2s, http) = (free_port(), free_port());
        let mut server = Server {
            bin,
            dir,
            child: None,
            c2s,
            http,
        };
        server.configure(true);
        server
    }

    fn configure(&mut self, smacks: bool) {
        let cfg = prosody_config(&self.dir, self.c2s, self.http, smacks);
        fs::write(self.dir.join("prosody.cfg.lua"), cfg).unwrap();
    }

    fn ctl(&self, args: &[&str]) {
        let ctl = self.bin.parent().unwrap().join("prosodyctl");
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
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
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
        }
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Read events until `f` accepts one. Panics with `what` on a timeout, so a failing check names itself.
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
            Ok(Some(ev)) => {
                if let Some(v) = f(&ev) {
                    return v;
                }
            }
            Ok(None) => panic!("{what}: the event channel closed"),
            Err(_) => panic!("{what}: timed out after {secs}s"),
        }
    }
}

fn pass(name: &str) {
    eprintln!("PASS {name}");
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

/// A started server with `alice` and `bob`, or `None` (the test is skipped) without `XAT_TEST_PROSODY_BIN`.
fn new_server() -> Option<Server> {
    let Some(bin) = std::env::var_os("XAT_TEST_PROSODY_BIN") else {
        eprintln!("skipped: XAT_TEST_PROSODY_BIN is not set");
        return None;
    };
    let mut server = Server::new(PathBuf::from(bin));
    for u in ["alice", "bob"] {
        server.ctl(&["register", u, "localhost", &format!("pw-{u}")]);
    }
    server.start();
    Some(server)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn against_a_real_prosody() {
    let Some(mut server) = new_server() else {
        return;
    };

    // 1. a verified TLS connection, SASL and bind
    let (alice, mut alice_rx) = connect(server.c2s, "alice").await;
    let (bob, mut bob_rx) = connect(server.c2s, "bob").await;
    pass("connect with a verified certificate");

    // 2. direct messages both ways
    let id = alice.send_chat("bob@localhost", "hello bob").await.unwrap();
    let from = expect(&mut bob_rx, 10, "dm to bob", |e| match e {
        Event::Chat {
            from,
            body,
            id: got,
            ..
        } if body == "hello bob" && got.as_deref() == Some(&id) => Some(from.clone()),
        _ => None,
    })
    .await;
    assert!(from.starts_with("alice@localhost/"), "{from}");
    bob.send_chat(&from, "hello alice").await.unwrap();
    expect(&mut alice_rx, 10, "dm to alice", |e| {
        matches!(e, Event::Chat { body, .. } if body == "hello alice").then_some(())
    })
    .await;
    bob.chat_state("alice@localhost", "composing")
        .await
        .unwrap();
    expect(&mut alice_rx, 10, "chat state", |e| {
        matches!(e, Event::ChatState { state, .. } if state == "composing").then_some(())
    })
    .await;
    pass("direct messages both ways, chat state");

    // 3. rooms: join, groupchat, the real JID, our own echo
    alice.join_room(ROOM, "alice").await.unwrap();
    bob.join_room(ROOM, "bob").await.unwrap();
    let id1 = alice.send_groupchat(ROOM, "hello room").await.unwrap();
    let (real, mid) = expect(&mut bob_rx, 10, "groupchat to bob", |e| match e {
        Event::Groupchat {
            room,
            nick,
            real_jid,
            body,
            id,
            from_self: false,
            ..
        } if body == "hello room" && room == ROOM && nick == "alice" => {
            Some((real_jid.clone(), id.clone()))
        }
        _ => None,
    })
    .await;
    assert!(
        real.as_deref()
            .is_some_and(|j| j.starts_with("alice@localhost/")),
        "real jid: {real:?}"
    );
    assert_eq!(
        mid.as_deref(),
        Some(id1.as_str()),
        "the room keeps the sender's message id"
    );
    expect(&mut alice_rx, 10, "own echo", |e| {
        matches!(e, Event::Groupchat { body, from_self: true, .. } if body == "hello room")
            .then_some(())
    })
    .await;
    pass("room join, groupchat, real JID visible, own message is from_self");

    // 4. correction and reaction
    let id2 = alice
        .correct(ROOM, true, &id1, "hello room (fixed)")
        .await
        .unwrap();
    let replaces = expect(&mut bob_rx, 10, "correction", |e| match e {
        Event::Groupchat {
            replaces: Some(r),
            body,
            ..
        } if body == "hello room (fixed)" => Some(r.clone()),
        _ => None,
    })
    .await;
    assert_eq!(replaces, id1);
    alice.react(ROOM, true, &id1, &["\u{1F44D}"]).await.unwrap();
    let (target, emojis) = expect(&mut bob_rx, 10, "reaction", |e| match e {
        Event::Reaction {
            target_id, emojis, ..
        } => Some((target_id.clone(), emojis.clone())),
        _ => None,
    })
    .await;
    assert_eq!(
        (target.as_str(), emojis),
        (id1.as_str(), vec!["\u{1F44D}".to_string()])
    );
    pass("correction and reaction");

    // 5. history: 7 bodied messages in the room (the reaction has no body, so it is not archived), two per page, newest first
    for i in 1..=5 {
        alice
            .send_groupchat(ROOM, &format!("filler {i}"))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let p1 = bob.history(Some(ROOM), true, 2, None).await.unwrap();
    assert_eq!(
        p1.iter().map(|a| a.body.as_str()).collect::<Vec<_>>(),
        ["filler 4", "filler 5"]
    );
    let p2 = bob
        .history(Some(ROOM), true, 2, Some(&p1[0].id))
        .await
        .unwrap();
    assert_eq!(
        p2.iter().map(|a| a.body.as_str()).collect::<Vec<_>>(),
        ["filler 2", "filler 3"]
    );
    let p3 = bob
        .history(Some(ROOM), true, 2, Some(&p2[0].id))
        .await
        .unwrap();
    assert_eq!(
        p3.iter().map(|a| a.body.as_str()).collect::<Vec<_>>(),
        ["hello room (fixed)", "filler 1"]
    );
    let all = bob.history(Some(ROOM), true, 50, None).await.unwrap();
    assert_eq!(all.len(), 7, "{all:?}");
    assert!(all[0].from.starts_with(ROOM) && !all[0].stamp.is_empty());
    let dm = bob
        .history(Some("alice@localhost"), false, 10, None)
        .await
        .unwrap();
    assert_eq!(
        dm.iter().map(|a| a.body.as_str()).collect::<Vec<_>>(),
        ["hello bob", "hello alice"]
    );
    let _ = id2;
    pass("history paging (room, three pages) and a direct-message archive");

    // 6. upload round trip
    let file = server.dir.join("payload.txt");
    fs::write(&file, b"payload 123").unwrap();
    let url = bob
        .upload_file("upload.localhost", &file, "text/plain")
        .await
        .unwrap();
    let got = reqwest::get(&url).await.unwrap().bytes().await.unwrap();
    assert_eq!(&got[..], b"payload 123");
    pass("upload slot, PUT and GET round trip");

    // 7. reconnect: kill the server, bring it back; the streams retry with a growing delay, so allow a while
    // stream management stays on, as in production: the old session is gone, so the resumption must fail cleanly
    server.stop();
    expect(&mut alice_rx, 20, "reconnecting", |e| {
        matches!(e, Event::Reconnecting).then_some(())
    })
    .await;
    server.start();
    expect(&mut alice_rx, 90, "alice reconnected", |e| {
        matches!(e, Event::Reconnected).then_some(())
    })
    .await;
    expect(&mut bob_rx, 90, "bob reconnected", |e| {
        matches!(e, Event::Reconnected).then_some(())
    })
    .await;
    alice
        .send_chat("bob@localhost", "after the restart")
        .await
        .unwrap();
    expect(&mut bob_rx, 10, "dm after restart", |e| {
        matches!(e, Event::Chat { body, .. } if body == "after the restart").then_some(())
    })
    .await;
    // both rejoined the room by themselves
    tokio::time::sleep(Duration::from_secs(1)).await;
    alice
        .send_groupchat(ROOM, "room after the restart")
        .await
        .unwrap();
    expect(&mut bob_rx, 10, "room after restart", |e| {
        matches!(e, Event::Groupchat { body, .. } if body == "room after the restart").then_some(())
    })
    .await;
    pass(
        "reconnect with stream management on: Reconnecting, Reconnected, a later send works, rooms rejoined",
    );

    alice.disconnect().await;
    bob.disconnect().await;
}

/// A TCP relay in front of the server that can cut every open connection while the server keeps running, which is a network
/// drop: the server still holds the stream-management session.
struct Proxy {
    port: u16,
    conns: std::sync::Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl Proxy {
    async fn start(upstream: u16) -> Proxy {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let conns = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let held = conns.clone();
        tokio::spawn(async move {
            while let Ok((mut down, _)) = listener.accept().await {
                let task = tokio::spawn(async move {
                    if let Ok(mut up) =
                        tokio::net::TcpStream::connect(("127.0.0.1", upstream)).await
                    {
                        let _ = tokio::io::copy_bidirectional(&mut down, &mut up).await;
                    }
                });
                held.lock().unwrap().push(task);
            }
        });
        Proxy { port, conns }
    }

    /// Drop every connection now; new ones are accepted again.
    fn cut(&self) {
        for task in self.conns.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_connection_resumes_the_session() {
    let Some(server) = new_server() else { return };
    let proxy = Proxy::start(server.c2s).await;
    let (alice, mut alice_rx) = connect(proxy.port, "alice").await;
    let (bob, _bob_rx) = connect(server.c2s, "bob").await;
    alice.join_room(ROOM, "alice").await.unwrap();

    proxy.cut();
    // sent while alice is away: stream management holds it for her
    bob.send_chat("alice@localhost", "while you were away")
        .await
        .unwrap();
    expect(&mut alice_rx, 30, "reconnecting", |e| {
        matches!(e, Event::Reconnecting).then_some(())
    })
    .await;
    expect(&mut alice_rx, 60, "reconnected", |e| {
        matches!(e, Event::Reconnected).then_some(())
    })
    .await;
    expect(&mut alice_rx, 10, "the message sent during the drop", |e| {
        matches!(e, Event::Chat { body, .. } if body == "while you were away").then_some(())
    })
    .await;
    alice.send_chat("bob@localhost", "back").await.unwrap();
    pass("a cut connection resumes with stream management on");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_password_fails_fast() {
    let Some(server) = new_server() else { return };
    let cfg = Config {
        jid: "alice@localhost".into(),
        password: "not the password".into(),
        server: Some(("localhost".into(), server.c2s)),
    };
    let started = Instant::now();
    let err = XmppClient::connect(cfg)
        .await
        .err()
        .expect("a wrong password must not connect");
    assert!(matches!(err, xmpp_agent_xmpp::Error::Auth(_)), "{err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "took {:?}",
        started.elapsed()
    );
    pass("a wrong password is refused quickly with Error::Auth");
}
