//! `xmpp-agent`: connect a human on XMPP to a coding agent over ACP, in direct messages.
//!
//! The pure parts ([`config`], [`split`], [`format`], [`log`] formatting) take their inputs as arguments and do no I/O. [`session::Session`]
//! is the loop: it owns the agent, the approvals broker and the conversation state, and talks to the chat through the [`chat::Chat`] trait,
//! so the tests drive it with an in-process fake and the real ACP session.

pub mod chat;
pub mod config;
pub mod format;
pub mod log;
pub mod session;
pub mod split;
