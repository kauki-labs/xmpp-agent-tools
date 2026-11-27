//! The real binary: exit codes, output channels and the stdio MCP stream.

mod support;

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Output, Stdio};

use support::{Fake, TOKEN};

const BIN: &str = env!("CARGO_BIN_EXE_xmpp-agent-rooms");

fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(BIN).args(args).env_clear().envs(env.iter().copied()).output().expect("spawn")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn create_should_print_the_result_and_exit_zero_when_the_service_answers_200() {
    let fake = Fake::always(200, r#"{"room": "lab@muc.example.org", "created": true}"#);
    let out = run(&["create", "lab"], &[("XAT_CAPABILITIES", "rooms"), ("XAT_RIGHTS_URL", &fake.url), ("XAT_RIGHTS_TOKEN", TOKEN)]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout).trim(), "created lab@muc.example.org");
    assert!(!text(&out.stdout).contains(TOKEN) && !text(&out.stderr).contains(TOKEN));
}

#[test]
fn assign_should_read_the_config_file_when_the_flag_is_given() {
    let fake = Fake::always(200, "{}");
    let dir = std::env::temp_dir().join(format!("xat-rooms-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let file = dir.join("rooms.toml");
    std::fs::write(&file, format!("rights_url = \"{}\"\nrights_token = \"{TOKEN}\"\n", fake.url)).expect("write");
    let out = run(&["--config", file.to_str().expect("utf8"), "assign", "lab", "alice@chat.example.org"], &[("XAT_CAPABILITIES", "rooms")]);
    std::fs::remove_dir_all(&dir).expect("clean");
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout).trim(), "assigned alice@chat.example.org in lab");
}

#[test]
fn create_should_exit_nonzero_and_say_why_when_the_capability_is_missing() {
    let fake = Fake::always(200, "{}");
    let out = run(&["create", "lab"], &[("XAT_RIGHTS_URL", &fake.url), ("XAT_RIGHTS_TOKEN", TOKEN)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("rooms"), "{}", text(&out.stderr));
    assert!(out.stdout.is_empty());
    assert!(fake.seen().is_empty());
}

#[test]
fn create_should_exit_nonzero_and_name_the_setting_when_the_url_is_missing() {
    let out = run(&["create", "lab"], &[("XAT_CAPABILITIES", "rooms"), ("XAT_RIGHTS_TOKEN", TOKEN)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("XAT_RIGHTS_URL"), "{}", text(&out.stderr));
    assert!(!text(&out.stderr).contains("panicked"), "{}", text(&out.stderr));
}

#[test]
fn create_should_exit_nonzero_when_the_service_refuses() {
    let fake = Fake::always(403, "");
    let out = run(&["create", "lab"], &[("XAT_CAPABILITIES", "rooms"), ("XAT_RIGHTS_URL", &fake.url), ("XAT_RIGHTS_TOKEN", TOKEN)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("403"), "{}", text(&out.stderr));
}

#[test]
fn the_binary_should_exit_with_a_usage_error_when_the_arguments_are_wrong() {
    let out = run(&["create"], &[]);
    assert_eq!(out.status.code(), Some(2));
}

fn mcp_exchange(env: &[(&str, &str)], requests: &[&str]) -> Vec<serde_json::Value> {
    let mut child = Command::new(BIN).arg("mcp").env_clear().envs(env.iter().copied()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("spawn");
    let mut stdin = child.stdin.take().expect("stdin");
    let mut lines = BufReader::new(child.stdout.take().expect("stdout")).lines();
    let mut replies = Vec::new();
    for request in requests {
        writeln!(stdin, "{request}").expect("write");
        stdin.flush().expect("flush");
        if request.contains("\"id\"") {
            let line = lines.next().expect("a reply").expect("read");
            replies.push(serde_json::from_str(&line).unwrap_or_else(|e| panic!("stdout must hold only JSON-RPC, got {line:?}: {e}")));
        }
    }
    drop(stdin);
    let _ = child.wait();
    replies
}

const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;
const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
const LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;

#[test]
fn mcp_should_list_no_tools_on_stdout_when_the_capability_is_missing() {
    let replies = mcp_exchange(&[], &[INITIALIZE, INITIALIZED, LIST]);
    assert_eq!(replies[1]["result"]["tools"].as_array().map(Vec::len), Some(0), "{replies:?}");
}

#[test]
fn mcp_should_list_three_tools_on_stdout_when_the_capability_is_granted() {
    let fake = Fake::always(200, "{}");
    let env = [("XAT_CAPABILITIES", "rooms"), ("XAT_RIGHTS_URL", fake.url.as_str()), ("XAT_RIGHTS_TOKEN", TOKEN)];
    let replies = mcp_exchange(&env, &[INITIALIZE, INITIALIZED, LIST]);
    assert_eq!(replies[1]["result"]["tools"].as_array().map(Vec::len), Some(3), "{replies:?}");
}

#[test]
fn mcp_should_exit_nonzero_when_the_capability_is_granted_but_the_settings_are_missing() {
    let out = run(&["mcp"], &[("XAT_CAPABILITIES", "rooms")]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("XAT_RIGHTS_URL"), "{}", text(&out.stderr));
}
