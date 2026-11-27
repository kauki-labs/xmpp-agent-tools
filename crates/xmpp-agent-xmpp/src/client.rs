//! The networked client: one background task owns the stream.

use std::path::Path;

use crate::{Archived, Error, Event};

/// How to connect. `server` is host and port of a direct-TLS endpoint; `None` looks up the `_xmpps-client` SRV record of the JID's
/// domain.
#[derive(Debug, Clone)]
pub struct Config {
    pub jid: String,
    pub password: String,
    pub server: Option<(String, u16)>,
}

/// A connected client. Clones share one connection.
#[derive(Clone)]
pub struct XmppClient {}

fn unsupported<T>() -> Result<T, Error> {
    Err(Error::Server("not implemented".into()))
}

impl XmppClient {
    pub async fn connect(
        _cfg: Config,
    ) -> Result<(XmppClient, tokio::sync::mpsc::Receiver<Event>), Error> {
        unsupported()
    }
    pub async fn send_chat(&self, _to: &str, _body: &str) -> Result<String, Error> {
        unsupported()
    }
    pub async fn send_groupchat(&self, _room: &str, _body: &str) -> Result<String, Error> {
        unsupported()
    }
    pub async fn correct(
        &self,
        _to: &str,
        _groupchat: bool,
        _replaces_id: &str,
        _new_body: &str,
    ) -> Result<String, Error> {
        unsupported()
    }
    pub async fn react(
        &self,
        _to: &str,
        _groupchat: bool,
        _target_id: &str,
        _emojis: &[&str],
    ) -> Result<(), Error> {
        unsupported()
    }
    pub async fn chat_state(&self, _to: &str, _state: &str) -> Result<(), Error> {
        unsupported()
    }
    pub async fn join_room(&self, _room: &str, _nick: &str) -> Result<(), Error> {
        unsupported()
    }
    pub async fn upload_file(
        &self,
        _upload_host: &str,
        _path: &Path,
        _content_type: &str,
    ) -> Result<String, Error> {
        unsupported()
    }
    pub async fn history(
        &self,
        _with: Option<&str>,
        _room: bool,
        _limit: usize,
        _before: Option<&str>,
    ) -> Result<Vec<Archived>, Error> {
        unsupported()
    }
    pub async fn disconnect(&self) {}
}
