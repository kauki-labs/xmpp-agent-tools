//! The rights service contract as pure functions: build a request, map a response.

use serde::Deserialize;

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

/// Build the request that creates a room.
pub fn create_request(name: &RoomName) -> Request {
    Request { path: String::new(), body: name.as_str().to_owned() }
}

/// Build the request that sets one member's affiliation.
pub fn assign_request(room: &RoomName, jid: &BareJid, affiliation: Affiliation) -> Request {
    let _ = (jid, affiliation);
    Request { path: room.as_str().to_owned(), body: String::new() }
}

/// Map the answer to a room creation.
pub fn map_create(status: u16, body: &str) -> Result<Created, ServiceError> {
    let _ = (status, body);
    Ok(Created { room: String::new(), created: false })
}

/// Map the answer to an affiliation change.
pub fn map_assign(status: u16, body: &str) -> Result<(), ServiceError> {
    let _ = (status, body);
    Ok(())
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
        assert_eq!(body(&request), json!({"jid": "alice@chat.example.org", "affiliation": "member"}));
    }

    #[test]
    fn assign_request_should_send_none_when_the_member_is_removed() {
        let room = RoomName::parse("lab").expect("valid");
        let jid = BareJid::parse("alice@chat.example.org").expect("valid");
        let request = assign_request(&room, &jid, Affiliation::None);
        assert_eq!(body(&request), json!({"jid": "alice@chat.example.org", "affiliation": "none"}));
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
        assert_eq!(map_create(200, CREATED), Ok(Created { room: "lab@muc.example.org".to_owned(), created: true }));
        assert_eq!(map_create(200, EXISTED), Ok(Created { room: "lab@muc.example.org".to_owned(), created: false }));
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
    fn the_mappers_should_map_each_status_when_the_service_does_not_answer_200(#[case] status: u16, #[case] body: &str, #[case] expected: ServiceError) {
        assert_eq!(map_create(status, body), Err(expected.clone()));
        assert_eq!(map_assign(status, body), Err(expected));
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
            Err(ServiceError::Conflict(text)) => assert_eq!(text.chars().count(), ERROR_TEXT_MAX_CHARS),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }
}
