//! The rights service contract as pure functions: build a request, map a response.

use serde::Deserialize;
use serde_json::{Value, json};
use xmpp_agent_common::redact::redact;

use crate::errors::ServiceError;
use crate::validate::{Affiliation, BareJid, RoomName};

/// Longest service text kept in an error, in characters.
pub const ERROR_TEXT_MAX_CHARS: usize = 200;

/// One POST request to the rights service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The path, starting with `/v1/`.
    pub path: String,
    /// The JSON body.
    pub body: String,
}

/// The answer to a room creation.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Created {
    /// The room JID, `<name>@<muc host>`.
    pub room: String,
    /// False when the room already existed.
    pub created: bool,
}

impl Created {
    /// Accept the answer only when `room` is `<name>@<host>` for the requested name and a host with no whitespace or control character.
    pub fn for_name(self, name: &RoomName) -> Result<Self, ServiceError> {
        let host = self
            .room
            .strip_prefix(name.as_str())
            .and_then(|rest| rest.strip_prefix('@'));
        match host {
            Some(host)
                if !host.is_empty()
                    && !host.contains(|c: char| c.is_whitespace() || c.is_control()) =>
            {
                Ok(self)
            }
            _ => Err(ServiceError::MalformedResponse),
        }
    }
}

/// Build the request that creates a room.
pub fn create_request(name: &RoomName) -> Request {
    Request {
        path: "/v1/rooms".to_owned(),
        body: json!({"name": name.as_str()}).to_string(),
    }
}

/// Build the request that sets one member's affiliation.
pub fn assign_request(room: &RoomName, jid: &BareJid, affiliation: Affiliation) -> Request {
    Request {
        path: format!("/v1/rooms/{}/members", room.as_str()),
        body: json!({"jid": jid.as_str(), "affiliation": affiliation.as_str()}).to_string(),
    }
}

/// Map the answer to a room creation.
pub fn map_create(status: u16, body: &str) -> Result<Created, ServiceError> {
    match status {
        200 => serde_json::from_str(body).map_err(|_| ServiceError::MalformedResponse),
        other => Err(refusal(other, body)),
    }
}

/// Map the answer to an affiliation change. The contract body is `{}`; anything else is not an answer this client understands.
pub fn map_assign(status: u16, body: &str) -> Result<(), ServiceError> {
    match status {
        200 => match serde_json::from_str::<Value>(body) {
            Ok(Value::Object(fields)) if fields.is_empty() => Ok(()),
            _ => Err(ServiceError::MalformedResponse),
        },
        other => Err(refusal(other, body)),
    }
}

/// The error for a status other than 200. Only `400` and `409` carry service text.
fn refusal(status: u16, body: &str) -> ServiceError {
    match status {
        400 => ServiceError::Invalid(service_text(body)),
        401 => ServiceError::Unauthorized,
        403 => ServiceError::Forbidden,
        409 => ServiceError::Conflict(service_text(body)),
        500..=599 => ServiceError::Server(status),
        other => ServiceError::UnexpectedStatus(other),
    }
}

/// The `error` text of an error body, redacted first and cut second so a secret cannot survive as a prefix.
fn service_text(body: &str) -> String {
    let text = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("error")?.as_str().map(str::to_owned))
        .filter(|t| !t.is_empty());
    match text {
        Some(text) => sanitize(&text),
        None => "no details".to_owned(),
    }
}

/// Remove secrets from text of an outside origin, then cut it to [`ERROR_TEXT_MAX_CHARS`].
pub fn sanitize(text: &str) -> String {
    redact(text).chars().take(ERROR_TEXT_MAX_CHARS).collect()
}

#[cfg(test)]
mod requests {
    use serde_json::{Value, json};

    use super::*;

    fn body(request: &Request) -> Value {
        serde_json::from_str(&request.body).expect("the body is JSON")
    }

    #[test]
    fn create_request_should_post_the_name_when_the_name_is_valid() {
        let request = create_request(&RoomName::parse("lab").expect("valid"));
        assert_eq!(request.path, "/v1/rooms");
        assert_eq!(body(&request), json!({"name": "lab"}));
    }

    #[test]
    fn assign_request_should_name_the_room_in_the_path_and_the_member_in_the_body() {
        let room = RoomName::parse("lab_7").expect("valid");
        let jid = BareJid::parse("alice@chat.example.org").expect("valid");
        let request = assign_request(&room, &jid, Affiliation::Member);
        assert_eq!(request.path, "/v1/rooms/lab_7/members");
        assert_eq!(
            body(&request),
            json!({"jid": "alice@chat.example.org", "affiliation": "member"})
        );
    }

    #[test]
    fn assign_request_should_send_none_when_the_member_is_removed() {
        let room = RoomName::parse("lab").expect("valid");
        let jid = BareJid::parse("alice@chat.example.org").expect("valid");
        let request = assign_request(&room, &jid, Affiliation::None);
        assert_eq!(
            body(&request),
            json!({"jid": "alice@chat.example.org", "affiliation": "none"})
        );
    }
}

#[cfg(test)]
mod responses {
    use rstest::rstest;

    use super::*;

    // The bodies below are the examples of docs/rights-service-api.md.
    const CREATED: &str = r#"{"room": "lab@muc.example.org", "created": true}"#;
    const EXISTED: &str = r#"{"room": "lab@muc.example.org", "created": false}"#;

    #[test]
    fn map_create_should_return_the_room_when_the_status_is_200() {
        assert_eq!(
            map_create(200, CREATED),
            Ok(Created {
                room: "lab@muc.example.org".to_owned(),
                created: true
            })
        );
        assert_eq!(
            map_create(200, EXISTED),
            Ok(Created {
                room: "lab@muc.example.org".to_owned(),
                created: false
            })
        );
    }

    #[rstest]
    #[case("")]
    #[case("not json")]
    #[case("{}")]
    #[case("[]")]
    #[case(r#"{"room": "lab@muc.example.org"}"#)]
    #[case(r#"{"created": true}"#)]
    #[case(r#"{"room": 1, "created": true}"#)]
    #[case(r#"{"room": "lab@muc.example.org", "created": "yes"}"#)]
    fn map_create_should_fail_closed_when_a_200_body_does_not_match(#[case] body: &str) {
        assert_eq!(map_create(200, body), Err(ServiceError::MalformedResponse));
    }

    #[rstest]
    #[case(r#"{"error": "denied"}"#)]
    #[case(r#"{"room": "lab@muc.example.org"}"#)]
    #[case(r#"{"ok": true}"#)]
    fn map_assign_should_fail_closed_when_a_200_object_is_not_empty(#[case] body: &str) {
        assert_eq!(map_assign(200, body), Err(ServiceError::MalformedResponse));
    }

    #[test]
    fn map_assign_should_succeed_when_the_status_is_200_and_the_body_is_an_object() {
        assert_eq!(map_assign(200, "{}"), Ok(()));
    }

    #[rstest]
    #[case("")]
    #[case("not json")]
    #[case("[]")]
    #[case("null")]
    #[case("true")]
    fn map_assign_should_fail_closed_when_a_200_body_is_not_an_object(#[case] body: &str) {
        assert_eq!(map_assign(200, body), Err(ServiceError::MalformedResponse));
    }

    #[rstest]
    #[case(400, r#"{"error": "name is reserved"}"#, ServiceError::Invalid("name is reserved".to_owned()))]
    #[case(400, "not json", ServiceError::Invalid("no details".to_owned()))]
    #[case(400, "{}", ServiceError::Invalid("no details".to_owned()))]
    #[case(401, "", ServiceError::Unauthorized)]
    #[case(401, r#"{"error": "x"}"#, ServiceError::Unauthorized)]
    #[case(403, "", ServiceError::Forbidden)]
    #[case(409, r#"{"error": "room is locked"}"#, ServiceError::Conflict("room is locked".to_owned()))]
    #[case(500, "", ServiceError::Server(500))]
    #[case(502, "", ServiceError::Server(502))]
    #[case(599, "", ServiceError::Server(599))]
    #[case(404, "", ServiceError::UnexpectedStatus(404))]
    #[case(301, "", ServiceError::UnexpectedStatus(301))]
    #[case(302, "", ServiceError::UnexpectedStatus(302))]
    #[case(201, CREATED, ServiceError::UnexpectedStatus(201))]
    #[case(204, "", ServiceError::UnexpectedStatus(204))]
    #[case(0, "", ServiceError::UnexpectedStatus(0))]
    #[case(600, "", ServiceError::UnexpectedStatus(600))]
    fn the_mappers_should_map_each_status_when_the_service_does_not_answer_200(
        #[case] status: u16,
        #[case] body: &str,
        #[case] expected: ServiceError,
    ) {
        assert_eq!(map_create(status, body), Err(expected.clone()));
        assert_eq!(map_assign(status, body), Err(expected));
    }

    #[rstest]
    #[case("lab@muc example.org")]
    #[case("lab@muc.example.org\u{7f}")]
    #[case("lab@muc.example.org\n")]
    fn created_should_refuse_a_host_with_whitespace_or_a_control_character_when_checked_for_the_name(
        #[case] room: &str,
    ) {
        let name = RoomName::parse("lab").expect("valid");
        let created = Created {
            room: room.to_owned(),
            created: true,
        };
        assert_eq!(
            created.for_name(&name),
            Err(ServiceError::MalformedResponse)
        );
    }

    #[test]
    fn map_create_should_redact_the_service_text_when_it_holds_a_secret() {
        let body = r#"{"error": "bad request, token=abcdef123456 was sent"}"#;
        match map_create(400, body) {
            Err(ServiceError::Invalid(text)) => assert!(!text.contains("abcdef123456"), "{text}"),
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn map_create_should_cut_the_service_text_when_it_is_long() {
        let body = format!(r#"{{"error": "{}"}}"#, "x".repeat(5000));
        match map_create(409, &body) {
            Err(ServiceError::Conflict(text)) => {
                assert_eq!(text.chars().count(), ERROR_TEXT_MAX_CHARS)
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }
}
