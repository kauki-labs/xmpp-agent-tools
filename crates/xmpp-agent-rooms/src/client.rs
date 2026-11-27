//! The HTTP side: send one request to the rights service and return the status and body.

use serde_json::Value;

use crate::api::sanitize;
use crate::config::Config;
use crate::errors::ServiceError;

/// Largest answer body read, in bytes. The contract's bodies are a few hundred bytes.
const MAX_BODY_BYTES: u64 = 64 * 1024;

/// Replace the secret in a body. A JSON body is decoded first, so an echo written with escapes (`\u0074ok`) is found as well.
fn scrub_json(body: &str, secret: &str) -> String {
    fn walk(value: &mut Value, secret: &str) {
        match value {
            Value::String(text) => *text = text.replace(secret, "[redacted]"),
            Value::Array(items) => items.iter_mut().for_each(|item| walk(item, secret)),
            Value::Object(fields) => fields.values_mut().for_each(|item| walk(item, secret)),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    match serde_json::from_str::<Value>(body) {
        Ok(mut value) => {
            walk(&mut value, secret);
            value.to_string()
        }
        Err(_) => body.replace(secret, "[redacted]"),
    }
}

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
        let secret = self.config.token.expose();
        let scrub = |text: String| sanitize(&text.replace(secret, "[redacted]"));
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.config.timeout))
            // A redirect would carry the bearer token to another host, and a 3xx is not an answer this contract knows.
            .max_redirects(0)
            // A proxy from the environment would see the bearer token.
            .proxy(None)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .build()
            .into();
        let sent = agent
            .post(format!("{}{path}", self.config.rights_url))
            .header("Authorization", format!("Bearer {secret}"))
            .header("Content-Type", "application/json")
            .send(body);
        match sent {
            Ok(mut response) => {
                let status = response.status().as_u16();
                match response
                    .body_mut()
                    .with_config()
                    .limit(MAX_BODY_BYTES)
                    .read_to_string()
                {
                    Ok(text) => Ok(Response {
                        status,
                        body: scrub_json(&text, secret),
                    }),
                    Err(e) => Err(ServiceError::Transport(scrub(e.to_string()))),
                }
            }
            Err(e) => Err(ServiceError::Transport(scrub(e.to_string()))),
        }
    }
}
