//! The XMPP layer of the agent client on `tokio-xmpp`.
//!
//! [`stanza`] is pure: it builds outgoing stanzas and turns incoming ones into [`Event`]s, with no I/O, so it is tested without a
//! network. [`XmppClient`] drives one connection in a background task and is cheap to clone. Reconnection is automatic: the
//! stream retries with a growing delay, [`Event::Reconnected`] follows each new session, and the rooms are joined again.

mod client;
mod login;
pub mod stanza;

use std::fmt;

pub use client::{Config, XmppClient};

/// Something that arrived from the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// A one-to-one message. `from` is the full JID; `replaces` is the id of the message this one corrects (XEP-0308).
    Chat {
        from: String,
        id: Option<String>,
        body: String,
        replaces: Option<String>,
    },
    /// A room message. `real_jid` is known only when the room shows real JIDs and the sender's presence was seen.
    Groupchat {
        room: String,
        nick: String,
        real_jid: Option<String>,
        id: Option<String>,
        body: String,
        replaces: Option<String>,
        from_self: bool,
    },
    /// A reaction (XEP-0444) to the message `target_id`; `from` is the full JID, or `room/nick` in a room.
    Reaction {
        from: String,
        target_id: String,
        emojis: Vec<String>,
    },
    /// A chat state notification (XEP-0085): `active`, `composing`, `paused`, `inactive` or `gone`.
    ChatState { from: String, state: String },
    /// A contact or room occupant went on or offline.
    Presence { from: String, available: bool },
    /// The connection dropped and the client is trying again.
    Reconnecting,
    /// A new session is up after a drop; rooms were joined again.
    Reconnected,
    /// The session ended for good.
    Disconnected(String),
}

/// One message from the archive (XEP-0313).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Archived {
    /// The archive id, which is what `before` of [`XmppClient::history`] takes to page backwards.
    pub id: String,
    pub from: String,
    pub body: String,
    /// When the message was sent, RFC 3339.
    pub stamp: String,
}

/// What can go wrong. Every request fails closed: a missing answer is [`Error::Timeout`], never a guess.
#[derive(Debug)]
pub enum Error {
    /// The caller passed a JID, state or value that cannot be sent.
    InvalidArgument(String),
    /// No answer in time; the string names the step.
    Timeout(&'static str),
    /// The client is gone or the session ended.
    Disconnected,
    /// The server refused the login: a wrong password or an unknown account.
    Auth(String),
    /// The server answered with an error.
    Server(String),
    /// The HTTP side of an upload failed.
    Http(String),
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidArgument(m) => write!(f, "invalid argument: {m}"),
            Error::Timeout(step) => write!(f, "timed out: {step}"),
            Error::Disconnected => write!(f, "disconnected"),
            Error::Auth(m) => write!(f, "login refused: {m}"),
            Error::Server(m) => write!(f, "server error: {m}"),
            Error::Http(m) => write!(f, "http error: {m}"),
            Error::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
