use std::path::Path;

use serde_json::{Value, json};

use super::*;

fn update(u: Value) -> Value {
    json!({"sessionId": "s1", "update": u})
}

#[test]
fn parses_a_success_response() {
    let m = parse_message(r#"{"jsonrpc":"2.0","id":3,"result":{"sessionId":"s1"}}"#).unwrap();
    assert_eq!(
        m,
        Incoming::Response {
            id: 3,
            result: Ok(json!({"sessionId": "s1"}))
        }
    );
}

#[test]
fn parses_an_error_response() {
    let m = parse_message(
        r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"Authentication required"}}"#,
    )
    .unwrap();
    assert_eq!(
        m,
        Incoming::Response {
            id: 4,
            result: Err((-32000, "Authentication required".into()))
        }
    );
}

#[test]
fn parses_a_notification() {
    let m =
        parse_message(r#"{"jsonrpc":"2.0","method":"session/update","params":{"a":1}}"#).unwrap();
    assert_eq!(
        m,
        Incoming::Notification {
            method: "session/update".into(),
            params: json!({"a": 1})
        }
    );
}

#[test]
fn parses_a_request_from_the_agent() {
    let m = parse_message(
        r#"{"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{}}"#,
    )
    .unwrap();
    assert_eq!(
        m,
        Incoming::Request {
            id: 7,
            method: "session/request_permission".into(),
            params: json!({})
        }
    );
}

#[test]
fn rejects_garbage_and_non_numeric_ids() {
    assert!(parse_message("not json").is_err());
    assert!(parse_message(r#"{"jsonrpc":"2.0"}"#).is_err());
    assert!(parse_message(r#"{"jsonrpc":"2.0","id":"x","method":"m"}"#).is_err());
    assert!(parse_message(r#"{"jsonrpc":"2.0","id":1}"#).is_err());
}

#[test]
fn decodes_text_and_thought_chunks() {
    let text = update(
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}),
    );
    assert_eq!(decode_update(&text), Some(AgentEvent::Text("hi".into())));
    let thought = update(
        json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"hmm"}}),
    );
    assert_eq!(
        decode_update(&thought),
        Some(AgentEvent::Thought("hmm".into()))
    );
}

#[test]
fn a_chunk_without_text_is_ignored() {
    let img = update(
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"image","data":"x"}}),
    );
    assert_eq!(decode_update(&img), None);
}

#[test]
fn decodes_a_tool_call() {
    let u = update(json!({
        "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Run ls",
        "kind": "execute", "status": "pending", "rawInput": {"command": "ls"}
    }));
    assert_eq!(
        decode_update(&u),
        Some(AgentEvent::ToolCall {
            id: "t1".into(),
            title: "Run ls".into(),
            kind: "execute".into(),
            raw_input: json!({"command": "ls"})
        })
    );
}

#[test]
fn a_tool_call_missing_fields_gets_defaults() {
    let u = update(json!({"sessionUpdate": "tool_call", "toolCallId": "t1"}));
    assert_eq!(
        decode_update(&u),
        Some(AgentEvent::ToolCall {
            id: "t1".into(),
            title: String::new(),
            kind: "other".into(),
            raw_input: Value::Null
        })
    );
}

#[test]
fn decodes_a_tool_update_with_text_content() {
    let u = update(json!({
        "sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed",
        "content": [
            {"type": "content", "content": {"type": "text", "text": "a"}},
            {"type": "diff", "path": "f", "newText": "x"},
            {"type": "content", "content": {"type": "text", "text": "b"}}
        ]
    }));
    assert_eq!(
        decode_update(&u),
        Some(AgentEvent::ToolUpdate {
            id: "t1".into(),
            status: "completed".into(),
            content: Some("a\nb".into())
        })
    );
}

#[test]
fn a_tool_update_without_content_or_status() {
    let u = update(json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1"}));
    assert_eq!(
        decode_update(&u),
        Some(AgentEvent::ToolUpdate {
            id: "t1".into(),
            status: String::new(),
            content: None
        })
    );
}

#[test]
fn decodes_available_commands() {
    let u = update(json!({
        "sessionUpdate": "available_commands_update",
        "availableCommands": [{"name": "compact", "description": "d"}, {"name": "init"}]
    }));
    assert_eq!(
        decode_update(&u),
        Some(AgentEvent::Commands(vec!["compact".into(), "init".into()]))
    );
}

#[test]
fn other_updates_are_ignored() {
    for kind in [
        "usage_update",
        "plan",
        "user_message_chunk",
        "current_mode_update",
        "nope",
    ] {
        assert_eq!(
            decode_update(&update(json!({"sessionUpdate": kind}))),
            None,
            "{kind}"
        );
    }
    assert_eq!(decode_update(&json!({})), None);
}

#[test]
fn decodes_a_permission_request() {
    let p = json!({
        "sessionId": "s1",
        "toolCall": {"toolCallId": "t1", "title": "touch a.txt", "kind": "execute", "rawInput": {"command": "touch a.txt"}},
        "options": [
            {"optionId": "allow_always", "name": "Always Allow", "kind": "allow_always"},
            {"optionId": "allow", "name": "Allow", "kind": "allow_once"},
            {"optionId": "reject", "name": "Reject", "kind": "reject_once"}
        ]
    });
    let ev = decode_permission(9, &p).unwrap();
    assert_eq!(
        ev,
        AgentEvent::Permission {
            request_id: 9,
            tool: "touch a.txt".into(),
            options: vec![
                PermOption {
                    id: "allow_always".into(),
                    name: "Always Allow".into(),
                    kind: "allow_always".into()
                },
                PermOption {
                    id: "allow".into(),
                    name: "Allow".into(),
                    kind: "allow_once".into()
                },
                PermOption {
                    id: "reject".into(),
                    name: "Reject".into(),
                    kind: "reject_once".into()
                },
            ]
        }
    );
}

#[test]
fn a_permission_request_without_tool_call_or_options_is_an_error() {
    assert!(decode_permission(1, &json!({"options": []})).is_err());
    assert!(decode_permission(1, &json!({"toolCall": {"title": "x"}})).is_err());
    let bad_option = json!({"toolCall": {"title": "x"}, "options": [{"name": "n"}]});
    assert!(decode_permission(1, &bad_option).is_err());
}

#[test]
fn reads_the_stop_reason() {
    assert_eq!(stop_reason(&json!({"stopReason": "end_turn"})), "end_turn");
    assert_eq!(stop_reason(&json!({})), "unknown");
}

#[test]
fn builds_the_handshake_requests() {
    let init = initialize_request(1);
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(init["id"], 1);
    assert_eq!(init["method"], "initialize");
    assert_eq!(init["params"]["protocolVersion"], 1);
    assert_eq!(init["params"]["clientCapabilities"], json!({}));
    assert_eq!(init["params"]["clientInfo"]["name"], "xmpp-agent-acp");

    let new = new_session_request(2, Path::new("/work"));
    assert_eq!(new["id"], 2);
    assert_eq!(new["method"], "session/new");
    assert_eq!(new["params"], json!({"cwd": "/work", "mcpServers": []}));

    let load = load_session_request(3, "s9", Path::new("/work"));
    assert_eq!(load["id"], 3);
    assert_eq!(load["method"], "session/load");
    assert_eq!(
        load["params"],
        json!({"sessionId": "s9", "cwd": "/work", "mcpServers": []})
    );
}

#[test]
fn builds_prompt_cancel_and_responses() {
    let p = prompt_request(5, "s1", "hello");
    assert_eq!(p["id"], 5);
    assert_eq!(p["method"], "session/prompt");
    assert_eq!(
        p["params"],
        json!({"sessionId": "s1", "prompt": [{"type": "text", "text": "hello"}]})
    );

    let c = cancel_notification("s1");
    assert_eq!(c["method"], "session/cancel");
    assert_eq!(c["params"], json!({"sessionId": "s1"}));
    assert!(c.get("id").is_none());
    assert_eq!(c["jsonrpc"], "2.0");

    let sel = permission_response(9, Some("allow"));
    assert_eq!(sel["id"], 9);
    assert_eq!(sel["jsonrpc"], "2.0");
    assert_eq!(
        sel["result"],
        json!({"outcome": {"outcome": "selected", "optionId": "allow"}})
    );
    let cancelled = permission_response(9, None);
    assert_eq!(
        cancelled["result"],
        json!({"outcome": {"outcome": "cancelled"}})
    );

    let e = error_response(4, -32601, "no such method");
    assert_eq!(e["id"], 4);
    assert_eq!(
        e["error"],
        json!({"code": -32601, "message": "no such method"})
    );
}
