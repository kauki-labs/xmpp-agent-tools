//! The networked client: one background task owns the stream; [`XmppClient`] talks to it over a channel.
//!
//! Every IQ is tracked here, not by `tokio-xmpp`, so that the `<fin/>` of an archive query is handled in stream order, after
//! the result messages that precede it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::StreamExt;
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;
use tokio_xmpp::connect::{DirectTlsServerConnector, DnsConfig};
use tokio_xmpp::stanzastream::{
    Event as StreamItem, StanzaStage, StanzaState, StanzaStream, StreamEvent,
};
use tokio_xmpp::xmlstream::Timeouts;
use tokio_xmpp::{Stanza, rustls};
use xmpp_parsers::http_upload::SlotResult;
use xmpp_parsers::iq::Iq;
use xmpp_parsers::jid::Jid;
use xmpp_parsers::message::Message;

use crate::stanza::{self, Parser};
use crate::{Archived, Error, Event};

/// The longest a request waits for its answer.
const TIMEOUT: Duration = Duration::from_secs(20);
/// The longest a first connection, with its login, may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const UPLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// How to connect. `server` is host and port of a direct-TLS endpoint; `None` looks up the `_xmpps-client` SRV record of the JID's
/// domain.
#[derive(Clone)]
pub struct Config {
    pub jid: String,
    pub password: String,
    pub server: Option<(String, u16)>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("jid", &self.jid)
            .field("password", &"[redacted]")
            .field("server", &self.server)
            .finish()
    }
}

/// An id that is unique within this process and unlikely to repeat across restarts.
fn next_id() -> String {
    static START: OnceLock<u128> = OnceLock::new();
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let start = START.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    });
    format!("xat-{start:x}-{}", COUNT.fetch_add(1, Ordering::Relaxed))
}

type Reply<T> = oneshot::Sender<Result<T, Error>>;

enum Pending {
    Iq(Reply<Iq>),
    History {
        queryid: String,
        scope: String,
        items: Vec<Archived>,
        reply: Reply<Vec<Archived>>,
    },
}

enum Cmd {
    Send(Stanza, Reply<()>),
    /// `scope` is the bare JID the result messages must come from; `None` means our own account.
    Iq(Iq, Pending, Option<String>),
    Join(String, String, Reply<()>),
    Disconnect(oneshot::Sender<()>),
}

/// A connected client. Clones share one connection.
#[derive(Clone)]
pub struct XmppClient {
    tx: mpsc::Sender<Cmd>,
}

impl XmppClient {
    /// Connect, log in and bind. The receiver must be read: a full channel stalls the connection.
    pub async fn connect(cfg: Config) -> Result<(XmppClient, mpsc::Receiver<Event>), Error> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let jid: Jid = cfg
            .jid
            .parse()
            .map_err(|e| Error::InvalidArgument(format!("jid {:?}: {e}", cfg.jid)))?;
        let dns = match &cfg.server {
            Some((host, port)) => DnsConfig::NoSrv {
                host: host.clone(),
                port: *port,
                resolver: None,
            },
            None => DnsConfig::srv_xmpps(jid.domain().as_ref()),
        };
        let stream = StanzaStream::new_c2s(
            DirectTlsServerConnector::from(dns),
            jid,
            cfg.password,
            Timeouts::default(),
            32,
        );
        let (events, events_rx) = mpsc::channel(256);
        let (cmd_tx, cmd_rx) = mpsc::channel(64);
        let (ready_tx, ready_rx) = oneshot::channel();
        let actor = Actor {
            stream,
            events,
            parser: Parser::new(),
            pending: HashMap::new(),
            joins: HashMap::new(),
            rooms: HashMap::new(),
            own_bare: String::new(),
            ready: Some(ready_tx),
        };
        tokio::spawn(actor.run(cmd_rx));
        // A wrong password or an unreachable server retries forever inside the stream; the timeout is what reports it.
        match timeout(CONNECT_TIMEOUT, ready_rx).await {
            Ok(Ok(())) => Ok((XmppClient { tx: cmd_tx }, events_rx)),
            Ok(Err(_)) => Err(Error::Disconnected),
            Err(_) => Err(Error::Timeout(
                "connect (bad credentials, an untrusted certificate or an unreachable server)",
            )),
        }
    }

    async fn ask<T>(
        &self,
        step: &'static str,
        make: impl FnOnce(Reply<T>) -> Cmd,
    ) -> Result<T, Error> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .send(make(tx))
            .await
            .map_err(|_| Error::Disconnected)?;
        match timeout(TIMEOUT, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(Error::Disconnected),
            Err(_) => Err(Error::Timeout(step)),
        }
    }

    async fn send_message(&self, m: Message) -> Result<(), Error> {
        self.ask("send", |r| Cmd::Send(Stanza::Message(m), r)).await
    }

    /// Send a one-to-one message and return its id.
    pub async fn send_chat(&self, to: &str, body: &str) -> Result<String, Error> {
        let id = next_id();
        self.send_message(stanza::chat(to, &id, body)?).await?;
        Ok(id)
    }

    pub async fn send_groupchat(&self, room: &str, body: &str) -> Result<String, Error> {
        let id = next_id();
        self.send_message(stanza::groupchat(room, &id, body)?)
            .await?;
        Ok(id)
    }

    /// Replace the message `replaces_id` and return the id of the correction.
    pub async fn correct(
        &self,
        to: &str,
        groupchat: bool,
        replaces_id: &str,
        new_body: &str,
    ) -> Result<String, Error> {
        let id = next_id();
        self.send_message(stanza::correction(
            to,
            groupchat,
            &id,
            replaces_id,
            new_body,
        )?)
        .await?;
        Ok(id)
    }

    pub async fn react(
        &self,
        to: &str,
        groupchat: bool,
        target_id: &str,
        emojis: &[&str],
    ) -> Result<(), Error> {
        self.send_message(stanza::reaction(
            to,
            groupchat,
            &next_id(),
            target_id,
            emojis,
        )?)
        .await
    }

    /// `active`, `composing`, `paused`, `inactive` or `gone`, to a one-to-one peer.
    pub async fn chat_state(&self, to: &str, state: &str) -> Result<(), Error> {
        self.send_message(stanza::chat_state(to, &next_id(), state)?)
            .await
    }

    /// Join an existing room and wait for the room to confirm. The room must already exist and be unlocked: this does not
    /// accept a default configuration. The client joins it again after a reconnect.
    pub async fn join_room(&self, room: &str, nick: &str) -> Result<(), Error> {
        stanza::join(room, nick)?; // validate before anything is recorded
        self.ask("join room", |r| {
            Cmd::Join(room.to_string(), nick.to_string(), r)
        })
        .await
    }

    /// Upload a file through the XEP-0363 service `upload_host` and return the URL to fetch it from.
    pub async fn upload_file(
        &self,
        upload_host: &str,
        path: &Path,
        content_type: &str,
    ) -> Result<String, Error> {
        let bytes = tokio::fs::read(path).await?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error::InvalidArgument("the path has no file name".into()))?;
        let iq = stanza::slot_request(
            upload_host,
            &next_id(),
            name,
            bytes.len() as u64,
            content_type,
        )?;
        let reply = self
            .ask("upload slot", |r| Cmd::Iq(iq, Pending::Iq(r), None))
            .await?;
        let slot = match reply {
            Iq::Result {
                payload: Some(el), ..
            } => SlotResult::try_from(el)
                .map_err(|e| Error::Server(format!("bad upload slot: {e}")))?,
            _ => return Err(Error::Server("empty upload slot".into())),
        };
        let http = reqwest::Client::builder()
            .timeout(UPLOAD_TIMEOUT)
            .build()
            .map_err(|e| Error::Http(e.to_string()))?;
        let mut put = http
            .put(&slot.put.url)
            .header("content-type", content_type)
            .body(bytes);
        for h in &slot.put.headers {
            put = put.header(h.name.as_str(), h.value.clone());
        }
        let status = put
            .send()
            .await
            .map_err(|e| Error::Http(e.to_string()))?
            .status();
        if !status.is_success() {
            return Err(Error::Http(format!("the upload was refused: {status}")));
        }
        Ok(slot.get.url)
    }

    /// The newest `limit` archived messages before the archive id `before` (the newest overall when `None`), oldest first.
    /// `with` is a peer's bare JID, or the room's when `room` is set. Page backwards by passing the first entry's id as `before`.
    pub async fn history(
        &self,
        with: Option<&str>,
        room: bool,
        limit: usize,
        before: Option<&str>,
    ) -> Result<Vec<Archived>, Error> {
        if limit == 0 {
            return Err(Error::InvalidArgument("limit must be at least 1".into()));
        }
        let (id, queryid) = (next_id(), next_id());
        let iq = stanza::history_query(&id, &queryid, with, room, limit, before)?;
        let scope = if room { with.map(str::to_string) } else { None };
        let step = "history";
        let (tx, rx) = oneshot::channel();
        let pending = Pending::History {
            queryid,
            scope: String::new(),
            items: Vec::new(),
            reply: tx,
        };
        self.tx
            .send(Cmd::Iq(iq, pending, scope))
            .await
            .map_err(|_| Error::Disconnected)?;
        match timeout(TIMEOUT, rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => Err(Error::Disconnected),
            Err(_) => Err(Error::Timeout(step)),
        }
    }

    /// Close the session. [`Event::Disconnected`] follows.
    pub async fn disconnect(&self) {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Cmd::Disconnect(tx)).await.is_ok() {
            let _ = timeout(TIMEOUT, rx).await;
        }
    }
}

struct Actor {
    stream: StanzaStream,
    events: mpsc::Sender<Event>,
    parser: Parser,
    pending: HashMap<String, Pending>,
    /// Joins waiting for the room's answer, keyed by room and nick.
    joins: HashMap<(String, String), Reply<()>>,
    /// Rooms to join again after a reconnect: room to nick.
    rooms: HashMap<String, String>,
    own_bare: String,
    /// Answered by the first session; `None` afterwards, which is how a later session is told from the first.
    ready: Option<oneshot::Sender<()>>,
}

fn iq_id(iq: &Iq) -> String {
    match iq {
        Iq::Get { id, .. } | Iq::Set { id, .. } | Iq::Result { id, .. } | Iq::Error { id, .. } => {
            id.clone()
        }
    }
}

impl Actor {
    async fn run(mut self, mut cmds: mpsc::Receiver<Cmd>) {
        let mut done = None;
        loop {
            tokio::select! {
                cmd = cmds.recv() => match cmd {
                    None => break,
                    Some(Cmd::Disconnect(d)) => { done = Some(d); break }
                    Some(cmd) => self.command(cmd).await,
                },
                item = self.stream.next() => match item {
                    None => break,
                    Some(item) => self.item(item).await,
                },
            }
        }
        let _ = timeout(Duration::from_secs(3), self.stream.close()).await;
        let _ = self.events.send(Event::Disconnected("closed".into())).await;
        if let Some(d) = done {
            let _ = d.send(());
        }
    }

    /// Queue a stanza; the token resolves when it was written or the stream gave up on it.
    async fn queue(&self, stanza: Stanza) -> tokio_xmpp::stanzastream::StanzaToken {
        self.stream.send(Box::new(stanza)).await
    }

    async fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Send(stanza, reply) => {
                let mut token = self.queue(stanza).await;
                tokio::spawn(async move {
                    let r = match timeout(TIMEOUT, token.wait_for(StanzaStage::Sent)).await {
                        Ok(Some(StanzaState::Sent { .. } | StanzaState::Acked { .. })) => Ok(()),
                        Ok(_) => Err(Error::Disconnected),
                        Err(_) => Err(Error::Timeout("send")),
                    };
                    let _ = reply.send(r);
                });
            }
            Cmd::Iq(iq, mut pending, scope) => {
                if let Pending::History { scope: s, .. } = &mut pending {
                    *s = scope.unwrap_or_else(|| self.own_bare.clone());
                }
                self.pending.insert(iq_id(&iq), pending);
                self.queue(Stanza::Iq(iq)).await;
            }
            Cmd::Join(room, nick, reply) => {
                let Ok(presence) = stanza::join(&room, &nick) else {
                    return;
                };
                self.parser.set_nick(&room, &nick);
                self.rooms.insert(room.clone(), nick.clone());
                self.joins.insert((room, nick), reply);
                self.queue(Stanza::Presence(presence)).await;
            }
            Cmd::Disconnect(_) => {}
        }
    }

    async fn emit(&self, ev: Event) {
        let _ = self.events.send(ev).await;
    }

    async fn item(&mut self, item: StreamItem) {
        match item {
            StreamItem::Stream(StreamEvent::Reset { bound_jid, .. }) => {
                self.own_bare = bound_jid.to_bare().to_string();
                // Requests of the old session will never be answered; dropping them reports `Disconnected` to their callers.
                self.pending.clear();
                self.queue(Stanza::Presence(stanza::initial_presence()))
                    .await;
                for (room, nick) in self.rooms.clone() {
                    if let Ok(p) = stanza::join(&room, &nick) {
                        self.queue(Stanza::Presence(p)).await;
                    }
                }
                match self.ready.take() {
                    Some(ready) => {
                        let _ = ready.send(());
                    }
                    None => self.emit(Event::Reconnected).await,
                }
            }
            StreamItem::Stream(StreamEvent::Suspended) => {
                if self.ready.is_none() {
                    self.emit(Event::Reconnecting).await;
                }
            }
            StreamItem::Stream(StreamEvent::Resumed) => self.emit(Event::Reconnected).await,
            StreamItem::Stanza(Stanza::Iq(iq)) => self.iq(iq).await,
            StreamItem::Stanza(Stanza::Message(m)) => {
                if let Some((queryid, entry)) = stanza::mam_result(&m) {
                    self.archive_result(&m, &queryid, entry);
                    return;
                }
                for ev in self.parser.handle(&Stanza::Message(m)) {
                    self.emit(ev).await;
                }
            }
            StreamItem::Stanza(Stanza::Presence(p)) => {
                let answered: Vec<_> = self
                    .joins
                    .keys()
                    .filter_map(|(room, nick)| {
                        stanza::join_result(&p, room, nick)
                            .map(|r| ((room.clone(), nick.clone()), r))
                    })
                    .collect();
                for (key, result) in answered {
                    if let Some(reply) = self.joins.remove(&key) {
                        let _ = reply.send(result.map_err(Error::Server));
                    }
                }
                for ev in self.parser.handle(&Stanza::Presence(p)) {
                    self.emit(ev).await;
                }
            }
        }
    }

    /// Keep an archive result only for a pending query and only from the account or room that was asked: anyone else could
    /// otherwise write into the history.
    fn archive_result(&mut self, m: &Message, queryid: &str, entry: Archived) {
        let sender = m.from.as_ref().map(|f| f.to_bare().to_string());
        for p in self.pending.values_mut() {
            if let Pending::History {
                queryid: q,
                scope,
                items,
                ..
            } = p
                && q == queryid
                && sender.as_ref().is_none_or(|s| s == scope)
            {
                items.push(entry);
                return;
            }
        }
    }

    async fn iq(&mut self, iq: Iq) {
        if let Some((id, _fin)) = stanza::mam_fin(&iq)
            && let Some(Pending::History { items, reply, .. }) = self.pending.remove(&id)
        {
            let _ = reply.send(Ok(items));
            return;
        }
        match iq {
            Iq::Result { .. } | Iq::Error { .. } => {
                let id = iq_id(&iq);
                match (self.pending.remove(&id), iq) {
                    (Some(Pending::Iq(reply)), iq @ Iq::Result { .. }) => {
                        let _ = reply.send(Ok(iq));
                    }
                    (Some(Pending::Iq(reply)), Iq::Error { error, .. }) => {
                        let _ = reply
                            .send(Err(Error::Server(format!("{:?}", error.defined_condition))));
                    }
                    (Some(Pending::History { reply, .. }), Iq::Error { error, .. }) => {
                        let _ = reply
                            .send(Err(Error::Server(format!("{:?}", error.defined_condition))));
                    }
                    (Some(Pending::History { reply, .. }), _) => {
                        let _ = reply.send(Err(Error::Server(
                            "the archive answered without a fin".into(),
                        )));
                    }
                    _ => {}
                }
            }
            request => {
                if let Some(reply) = stanza::iq_reply(&request) {
                    self.queue(Stanza::Iq(reply)).await;
                }
            }
        }
    }
}
