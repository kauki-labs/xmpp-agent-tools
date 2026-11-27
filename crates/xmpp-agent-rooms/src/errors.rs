//! Error types. Every failure is a value; an unknown outcome is never success.

use std::fmt;

/// An input the client refuses before it sends any request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// The room name does not match `[a-z0-9_]{1,32}`.
    RoomName,
    /// The room name starts with `org_`, which is reserved.
    OrgPrefix,
    /// The member JID is not a bare JID.
    Jid,
    /// The affiliation is not `member` or `none`.
    Affiliation,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::RoomName => "room name must match [a-z0-9_]{1,32}",
            Self::OrgPrefix => "room names starting with org_ are reserved",
            Self::Jid => {
                "member must be a bare JID: <local>@<domain>, no resource, no slash, no whitespace"
            }
            Self::Affiliation => "affiliation must be member or none",
        })
    }
}

impl std::error::Error for ValidationError {}

/// A failed or unusable answer from the rights service. Text from the service is already redacted and cut.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    /// `400`: the service refused the name or JID.
    Invalid(String),
    /// `401`: the token is missing, malformed or unknown.
    Unauthorized,
    /// `403`: the caller is not a higher entity.
    Forbidden,
    /// `409`: the request conflicts with the room state.
    Conflict(String),
    /// `5xx`: the service failed.
    Server(u16),
    /// Any other status.
    UnexpectedStatus(u16),
    /// A `200` whose body does not match the contract.
    MalformedResponse,
    /// No answer: connection failure or timeout. The text holds no secret.
    Transport(String),
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(text) => write!(f, "rights service refused the input: {text}"),
            Self::Unauthorized => f.write_str("rights service refused the token (401)"),
            Self::Forbidden => f.write_str("the caller is not a higher entity (403)"),
            Self::Conflict(text) => write!(f, "conflict: {text}"),
            Self::Server(status) => write!(f, "rights service failed ({status})"),
            Self::UnexpectedStatus(status) => {
                write!(f, "unexpected status {status} from the rights service")
            }
            Self::MalformedResponse => {
                f.write_str("the rights service sent a response that does not match the contract")
            }
            Self::Transport(text) => write!(f, "no answer from the rights service: {text}"),
        }
    }
}

impl std::error::Error for ServiceError {}

/// A setting that is missing or unusable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A required setting is absent. The text names the env variable.
    Missing(&'static str),
    /// The `--config` file cannot be read or parsed. The text holds no secret.
    File(String),
    /// The rights service URL is not an `http://` or `https://` URL.
    Url,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(name) => write!(f, "missing setting: set {name} or use --config"),
            Self::File(text) => write!(f, "cannot use the config file: {text}"),
            Self::Url => f.write_str("the rights service URL must start with http:// or https://"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Everything an action can fail with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomsError {
    /// The `rooms` capability is not granted.
    Disabled,
    /// The input failed validation; no request was sent.
    Validation(ValidationError),
    /// The settings are missing or unusable.
    Config(ConfigError),
    /// The rights service refused or failed.
    Service(ServiceError),
}

impl fmt::Display for RoomsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => {
                f.write_str("the rooms capability is not granted: add rooms to XAT_CAPABILITIES")
            }
            Self::Validation(e) => e.fmt(f),
            Self::Config(e) => e.fmt(f),
            Self::Service(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RoomsError {}

impl From<ValidationError> for RoomsError {
    fn from(e: ValidationError) -> Self {
        Self::Validation(e)
    }
}

impl From<ConfigError> for RoomsError {
    fn from(e: ConfigError) -> Self {
        Self::Config(e)
    }
}

impl From<ServiceError> for RoomsError {
    fn from(e: ServiceError) -> Self {
        Self::Service(e)
    }
}

#[cfg(test)]
mod display {
    use super::*;

    #[test]
    fn service_error_should_show_the_status_when_the_service_refused_with_text() {
        assert!(
            ServiceError::Invalid("x".to_owned())
                .to_string()
                .contains("400")
        );
        assert!(
            ServiceError::Conflict("x".to_owned())
                .to_string()
                .contains("409")
        );
    }
}
