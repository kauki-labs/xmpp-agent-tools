//! The chat side as a trait, so the loop is tested without a server.

use std::fmt;
use std::future::Future;

use xmpp_agent_xmpp::XmppClient;

/// A send that failed. The text holds no secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatError(pub String);

impl fmt::Display for ChatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ChatError {}

/// What the loop needs from the chat connection.
pub trait Chat: Send + Sync + 'static {
    /// Send a one-to-one message to a bare JID.
    fn send(&self, to: &str, body: &str) -> impl Future<Output = Result<(), ChatError>> + Send;

    /// Send a chat state (`composing`, `active`, ...) to a bare JID.
    fn state(&self, to: &str, state: &str) -> impl Future<Output = Result<(), ChatError>> + Send;
}

impl Chat for XmppClient {
    async fn send(&self, to: &str, body: &str) -> Result<(), ChatError> {
        self.send_chat(to, body)
            .await
            .map(|_| ())
            .map_err(|e| ChatError(e.to_string()))
    }

    async fn state(&self, to: &str, state: &str) -> Result<(), ChatError> {
        self.chat_state(to, state)
            .await
            .map_err(|e| ChatError(e.to_string()))
    }
}
