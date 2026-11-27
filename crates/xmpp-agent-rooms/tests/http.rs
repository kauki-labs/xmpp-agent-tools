//! The CLI actions end to end against the in-process fake rights service.

mod support;

use std::time::Duration;

use rstest::rstest;
use serde_json::{Value, json};
use support::{Fake, Reply, TOKEN};
use xmpp_agent_rooms::client::HttpTransport;
use xmpp_agent_rooms::config::{Capabilities, Config, Env};
use xmpp_agent_rooms::errors::{ConfigError, RoomsError, ServiceError, ValidationError};
use xmpp_agent_rooms::tools::{Action, Rooms};

fn create(name: &str) -> Action {
    Action::Create { name: name.to_owned() }
}

fn assign(room: &str, jid: &str) -> Action {
    Action::Assign { room: room.to_owned(), jid: jid.to_owned() }
}

fn unassign(room: &str, jid: &str) -> Action {
    Action::Unassign { room: room.to_owned(), jid: jid.to_owned() }
}

fn json_body(text: &str) -> Value {
    serde_json::from_str(text).expect("JSON body")
}

#[test]
fn create_should_post_the_name_with_the_bearer_token_when_the_service_answers_200() -> anyhow_free::Result {
    let fake = Fake::always(200, r#"{"room": "lab@muc.example.org", "created": true}"#);
    let line = fake.rooms().run(&create("lab"))?;
    assert_eq!(line, "created lab@muc.example.org");
    let seen = fake.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("POST", "/v1/rooms"));
    assert_eq!(seen[0].authorization.as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
    assert_eq!(json_body(&seen[0].body), json!({"name": "lab"}));
    Ok(())
}

#[test]
fn create_should_say_the_room_exists_when_created_is_false() -> anyhow_free::Result {
    let fake = Fake::always(200, r#"{"room": "lab@muc.example.org", "created": false}"#);
    assert_eq!(fake.rooms().run(&create("lab"))?, "lab@muc.example.org already exists");
    Ok(())
}

#[test]
fn assign_should_post_member_when_the_service_answers_200() -> anyhow_free::Result {
    let fake = Fake::always(200, "{}");
    let line = fake.rooms().run(&assign("lab", "alice@chat.example.org"))?;
    assert_eq!(line, "assigned alice@chat.example.org in lab");
    let seen = fake.seen();
    assert_eq!(seen[0].path, "/v1/rooms/lab/members");
    assert_eq!(seen[0].authorization.as_deref(), Some(format!("Bearer {TOKEN}").as_str()));
    assert_eq!(json_body(&seen[0].body), json!({"jid": "alice@chat.example.org", "affiliation": "member"}));
    Ok(())
}

#[test]
fn unassign_should_post_none_when_the_service_answers_200() -> anyhow_free::Result {
    let fake = Fake::always(200, "{}");
    let line = fake.rooms().run(&unassign("lab", "alice@chat.example.org"))?;
    assert_eq!(line, "unassigned alice@chat.example.org in lab");
    assert_eq!(json_body(&fake.seen()[0].body), json!({"jid": "alice@chat.example.org", "affiliation": "none"}));
    Ok(())
}

#[rstest]
#[case(400, r#"{"error": "bad name"}"#, ServiceError::Invalid("bad name".to_owned()))]
#[case(401, "", ServiceError::Unauthorized)]
#[case(403, "", ServiceError::Forbidden)]
#[case(409, r#"{"error": "locked"}"#, ServiceError::Conflict("locked".to_owned()))]
#[case(500, "boom", ServiceError::Server(500))]
#[case(503, "", ServiceError::Server(503))]
#[case(404, "", ServiceError::UnexpectedStatus(404))]
#[case(204, "", ServiceError::UnexpectedStatus(204))]
fn actions_should_map_the_status_when_the_service_refuses(#[case] status: u16, #[case] body: &str, #[case] expected: ServiceError) {
    let fake = Fake::always(status, body);
    assert_eq!(fake.rooms().run(&create("lab")), Err(RoomsError::Service(expected.clone())));
    assert_eq!(fake.rooms().run(&assign("lab", "alice@chat.example.org")), Err(RoomsError::Service(expected)));
}

#[test]
fn actions_should_fail_closed_when_a_200_body_is_malformed() {
    let fake = Fake::always(200, "<html>captive portal</html>");
    assert_eq!(fake.rooms().run(&create("lab")), Err(RoomsError::Service(ServiceError::MalformedResponse)));
    assert_eq!(fake.rooms().run(&unassign("lab", "alice@chat.example.org")), Err(RoomsError::Service(ServiceError::MalformedResponse)));
}

#[test]
fn actions_should_not_follow_a_redirect_when_the_service_sends_one() {
    let target = Fake::always(200, r#"{"room": "lab@muc.example.org", "created": true}"#);
    let location = format!("{}/v1/rooms", target.url);
    let fake = Fake::start(move |_| Reply { location: Some(location.clone()), ..Reply::new(307, "") });
    assert_eq!(fake.rooms().run(&create("lab")), Err(RoomsError::Service(ServiceError::UnexpectedStatus(307))));
    assert!(target.seen().is_empty(), "the redirect target received {:?}", target.seen());
}

#[test]
fn actions_should_fail_when_the_service_does_not_answer_in_time() {
    let fake = Fake::start(|_| Reply { delay: Duration::from_secs(2), ..Reply::new(200, "{}") });
    let rooms = fake.rooms_with(Capabilities::parse("rooms"), Duration::from_millis(200));
    assert!(matches!(rooms.run(&assign("lab", "alice@chat.example.org")), Err(RoomsError::Service(ServiceError::Transport(_)))));
}

#[test]
fn actions_should_fail_when_nothing_listens_and_the_error_should_not_hold_the_token() {
    let fake = Fake::always(200, "{}");
    let env = Env { rights_url: Some(fake.url.clone()), token: Some(TOKEN.to_owned()), capabilities: None };
    drop(fake);
    let rooms = Rooms::new(HttpTransport::new(Config::load(&env, None).expect("config")), Capabilities::parse("rooms"));
    let error = rooms.run(&create("lab")).expect_err("nothing listens");
    assert!(matches!(error, RoomsError::Service(ServiceError::Transport(_))), "{error:?}");
    assert!(!error.to_string().contains(TOKEN), "{error}");
}

#[test]
fn actions_should_not_show_the_token_when_the_service_echoes_it() {
    let reply = Reply::new(400, &format!(r#"{{"error": "bad credential {TOKEN} rejected"}}"#));
    let fake = Fake::start(move |_| reply.clone());
    let error = fake.rooms().run(&create("lab")).expect_err("400");
    assert!(!error.to_string().contains(TOKEN), "{error}");
    assert!(!format!("{error:?}").contains(TOKEN), "{error:?}");
}

#[rstest]
#[case(create(""))]
#[case(create("Lab"))]
#[case(create("org_staff"))]
#[case(assign("lab", "alice"))]
#[case(assign("lab", "alice@chat.example.org/phone"))]
#[case(assign("org_staff", "alice@chat.example.org"))]
#[case(unassign("lab\n", "alice@chat.example.org"))]
#[case(unassign("lab", "a b@chat.example.org"))]
fn actions_should_send_no_request_when_an_input_is_invalid(#[case] action: Action) {
    let fake = Fake::always(200, "{}");
    let error = fake.rooms().run(&action).expect_err("invalid input");
    assert!(matches!(error, RoomsError::Validation(_)), "{error:?}");
    assert!(fake.seen().is_empty(), "sent {:?}", fake.seen());
}

#[test]
fn actions_should_name_the_rule_when_the_room_uses_the_reserved_prefix() {
    let fake = Fake::always(200, "{}");
    assert_eq!(fake.rooms().run(&create("org_staff")), Err(RoomsError::Validation(ValidationError::OrgPrefix)));
}

#[rstest]
#[case("")]
#[case("directory")]
#[case("chatrooms")]
fn actions_should_run_nothing_when_the_rooms_capability_is_missing(#[case] capabilities: &str) {
    let fake = Fake::always(200, r#"{"room": "lab@muc.example.org", "created": true}"#);
    let rooms = fake.rooms_with(Capabilities::parse(capabilities), Duration::from_secs(5));
    for action in [create("lab"), assign("lab", "alice@chat.example.org"), unassign("lab", "alice@chat.example.org")] {
        assert_eq!(rooms.run(&action), Err(RoomsError::Disabled));
    }
    assert!(fake.seen().is_empty());
}

#[test]
fn rooms_should_fail_before_reading_settings_when_the_capability_is_missing() {
    let result = xmpp_agent_rooms::app::rooms(None, &Env::default(), None);
    assert!(matches!(result, Err(RoomsError::Disabled)), "{:?}", result.err());
}

#[test]
fn rooms_should_name_the_missing_setting_when_the_capability_is_granted_but_the_url_is_not_set() {
    let env = Env { rights_url: None, token: Some(TOKEN.to_owned()), capabilities: Some("rooms".to_owned()) };
    let result = xmpp_agent_rooms::app::rooms(None, &env, None);
    assert!(matches!(result, Err(RoomsError::Config(ConfigError::Missing("XAT_RIGHTS_URL")))), "{:?}", result.err());
}

#[test]
fn rooms_should_use_the_flag_when_the_environment_grants_nothing() -> anyhow_free::Result {
    let fake = Fake::always(200, r#"{"room": "lab@muc.example.org", "created": true}"#);
    let env = Env { capabilities: None, ..fake.env() };
    let rooms = xmpp_agent_rooms::app::rooms(Some("rooms"), &env, None)?;
    assert_eq!(rooms.run(&create("lab"))?, "created lab@muc.example.org");
    Ok(())
}

/// A tiny stand-in for `anyhow`, which the workspace does not use: tests return this so `?` works on the crate's errors.
mod anyhow_free {
    pub type Result = std::result::Result<(), Box<dyn std::error::Error>>;
}
