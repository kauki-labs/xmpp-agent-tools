//! The HTTP side: send one request to the rights service and return the status and body.

use crate::config::Config;
use crate::errors::ServiceError;

/// A status code and body, before any mapping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The HTTP status.
    pub status: u16,
    /// The body text.
    pub body: String,
}

/// Something that can POST a JSON body to the rights service. Tests replace it; the MCP server runs it on a blocking thread.
pub trait Transport {
    /// POST `body` to `path` and return the answer, or a [`ServiceError::Transport`].
    fn post(&self, path: &str, body: &str) -> Result<Response, ServiceError>;
}

/// The real transport: `ureq` with a bearer token and a timeout.
#[derive(Debug)]
pub struct HttpTransport {
    config: Config,
}

impl HttpTransport {
    /// Build a transport for these settings.
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

impl Transport for HttpTransport {
    fn post(&self, path: &str, body: &str) -> Result<Response, ServiceError> {
        let _ = (path, body, &self.config);
        Err(ServiceError::Transport("not implemented".to_owned()))
    }
}
