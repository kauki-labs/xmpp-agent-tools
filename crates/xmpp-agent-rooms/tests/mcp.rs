//! The MCP server in process: a real rmcp client talks to it over an in-memory pipe.

mod support;

use std::borrow::Cow;

use rmcp::ServiceExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, ContentBlock,
    Implementation,
};
use serde_json::{Map, Value, json};
use support::{Fake, TOKEN};
use xmpp_agent_rooms::app;
use xmpp_agent_rooms::client::HttpTransport;
use xmpp_agent_rooms::config::Env;
use xmpp_agent_rooms::mcp::RoomsServer;

type Client = rmcp::service::RunningService<rmcp::RoleClient, ClientConfig>;

async fn connect(server: RoomsServer<HttpTransport>) -> Client {
    let (server_end, client_end) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = server.serve(server_end).await.expect("server starts");
        let _ = running.waiting().await;
    });
    let info = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("test-client", "0"),
    );
    info.serve(client_end).await.expect("client starts")
}

fn arguments(value: Value) -> Map<String, Value> {
    let Value::Object(map) = value else {
        panic!("an object")
    };
    map
}

fn call(name: &'static str, args: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(Cow::Borrowed(name)).with_arguments(arguments(args))
}

fn text_of(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            ContentBlock::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn enabled(fake: &Fake) -> RoomsServer<HttpTransport> {
    app::mcp_server(None, &fake.env(), None).expect("server")
}

#[tokio::test]
async fn list_tools_should_return_the_three_room_tools_when_rooms_is_granted() {
    let fake = Fake::always(200, "{}");
    let client = connect(enabled(&fake)).await;
    let mut names: Vec<String> = client
        .list_all_tools()
        .await
        .expect("list")
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["room_assign", "room_create", "room_unassign"]);
}

#[tokio::test]
async fn list_tools_should_describe_each_argument_as_a_required_string() {
    let fake = Fake::always(200, "{}");
    let client = connect(enabled(&fake)).await;
    for tool in client.list_all_tools().await.expect("list") {
        let schema = Value::Object((*tool.input_schema).clone());
        let required: Vec<&str> = schema["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let expected: &[&str] = if tool.name == "room_create" {
            &["name"]
        } else {
            &["room", "jid"]
        };
        assert_eq!(required, expected, "{}", tool.name);
        for key in expected {
            assert_eq!(
                schema["properties"][key]["type"], "string",
                "{} {key}",
                tool.name
            );
        }
    }
}

#[tokio::test]
async fn list_tools_should_return_nothing_when_rooms_is_not_granted() {
    let env = Env {
        capabilities: None,
        ..Fake::always(200, "{}").env()
    };
    let client =
        connect(app::mcp_server(None, &env, None).expect("a disabled server needs no settings"))
            .await;
    assert!(client.list_all_tools().await.expect("list").is_empty());
}

#[tokio::test]
async fn call_tool_should_refuse_and_send_nothing_when_rooms_is_not_granted() {
    let fake = Fake::always(200, r#"{"room": "lab@muc.example.org", "created": true}"#);
    let env = Env {
        capabilities: Some("directory".to_owned()),
        ..fake.env()
    };
    let client = connect(app::mcp_server(None, &env, None).expect("server")).await;
    let outcome = client
        .call_tool(call("room_create", json!({"name": "lab"})))
        .await;
    // A protocol error is as good a refusal as a tool error; both must leave the service untouched.
    if let Ok(result) = outcome {
        assert_eq!(result.is_error, Some(true), "{}", text_of(&result));
    }
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn call_tool_should_create_a_room_when_the_service_answers_200() {
    let fake = Fake::always(200, r#"{"room": "lab@muc.example.org", "created": true}"#);
    let client = connect(enabled(&fake)).await;
    let result = client
        .call_tool(call("room_create", json!({"name": "lab"})))
        .await
        .expect("call");
    assert_ne!(result.is_error, Some(true));
    assert_eq!(text_of(&result), "created lab@muc.example.org");
    assert_eq!(fake.seen()[0].path, "/v1/rooms");
}

#[tokio::test]
async fn call_tool_should_assign_and_unassign_when_the_service_answers_200() {
    let fake = Fake::always(200, "{}");
    let client = connect(enabled(&fake)).await;
    let assigned = client
        .call_tool(call(
            "room_assign",
            json!({"room": "lab", "jid": "alice@chat.example.org"}),
        ))
        .await
        .expect("call");
    assert_eq!(text_of(&assigned), "assigned alice@chat.example.org in lab");
    let removed = client
        .call_tool(call(
            "room_unassign",
            json!({"room": "lab", "jid": "alice@chat.example.org"}),
        ))
        .await
        .expect("call");
    assert_eq!(
        text_of(&removed),
        "unassigned alice@chat.example.org in lab"
    );
    let bodies: Vec<Value> = fake
        .seen()
        .iter()
        .map(|s| serde_json::from_str(&s.body).expect("JSON"))
        .collect();
    assert_eq!(bodies[0]["affiliation"], "member");
    assert_eq!(bodies[1]["affiliation"], "none");
}

#[tokio::test]
async fn call_tool_should_return_a_tool_error_when_the_service_refuses() {
    let fake = Fake::always(403, "");
    let client = connect(enabled(&fake)).await;
    let result = client
        .call_tool(call("room_create", json!({"name": "lab"})))
        .await
        .expect("call");
    assert_eq!(result.is_error, Some(true));
    assert!(text_of(&result).contains("403"), "{}", text_of(&result));
}

#[tokio::test]
async fn call_tool_should_return_a_tool_error_and_send_nothing_when_the_input_is_invalid() {
    let fake = Fake::always(200, "{}");
    let client = connect(enabled(&fake)).await;
    let result = client
        .call_tool(call("room_create", json!({"name": "org_staff"})))
        .await
        .expect("call");
    assert_eq!(result.is_error, Some(true));
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn call_tool_should_return_a_tool_error_when_an_argument_is_missing_or_not_a_string() {
    let fake = Fake::always(200, "{}");
    let client = connect(enabled(&fake)).await;
    for args in [json!({}), json!({"name": 7}), json!({"nom": "lab"})] {
        let result = client
            .call_tool(call("room_create", args))
            .await
            .expect("call");
        assert_eq!(result.is_error, Some(true));
    }
    let result = client
        .call_tool(call("room_assign", json!({"room": "lab"})))
        .await
        .expect("call");
    assert_eq!(result.is_error, Some(true));
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn call_tool_should_fail_when_the_tool_is_unknown() {
    let fake = Fake::always(200, "{}");
    let client = connect(enabled(&fake)).await;
    assert!(
        client
            .call_tool(call("room_delete", json!({"name": "lab"})))
            .await
            .is_err()
    );
    assert!(fake.seen().is_empty());
}

#[tokio::test]
async fn call_tool_should_not_show_the_token_when_the_service_echoes_it() {
    let fake = Fake::always(
        400,
        &format!(r#"{{"error": "bad credential {TOKEN} rejected"}}"#),
    );
    let client = connect(enabled(&fake)).await;
    let result = client
        .call_tool(call("room_create", json!({"name": "lab"})))
        .await
        .expect("call");
    assert_eq!(result.is_error, Some(true));
    assert!(!text_of(&result).contains(TOKEN), "{}", text_of(&result));
    let listed = format!("{:?}", client.list_all_tools().await.expect("list"));
    assert!(!listed.contains(TOKEN));
}
