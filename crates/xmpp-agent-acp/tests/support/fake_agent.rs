//! A scripted ACP agent for the integration tests. It speaks newline-delimited JSON-RPC on stdio and is driven by the prompt text:
//!
//! - `hello`: a thought, two text chunks, a tool call and its update, then `end_turn`
//! - `ask`: a permission request; the text chunk reports the outcome (`allowed:<id>`, `rejected:<id>` or `denied` for a cancelled outcome), then `end_turn`
//! - `slow`: text chunks until `session/cancel`, then `stopReason: cancelled`
//! - `die`: write to stderr and exit with status 3
//!
//! `session/new` answers `fake-1` and sends two commands; `session/load` knows only `fake-1` and replays two updates.

use std::io::{BufRead, Write};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use serde_json::{Value, json};

fn send(v: Value) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{v}").unwrap();
    out.flush().unwrap();
}

fn update(u: Value) {
    send(
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"fake-1","update":u}}),
    );
}

fn text(t: &str) {
    update(json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":t}}));
}

fn reply(id: &Value, result: Value) {
    send(json!({"jsonrpc":"2.0","id":id,"result":result}));
}

fn next(rx: &Receiver<Value>, wait: Duration) -> Option<Value> {
    match rx.recv_timeout(wait) {
        Ok(v) => Some(v),
        Err(RecvTimeoutError::Timeout) => None,
        Err(RecvTimeoutError::Disconnected) => std::process::exit(0),
    }
}

fn turn(id: &Value, prompt: &str, rx: &Receiver<Value>) {
    let mut stop = "end_turn";
    match prompt {
        "hello" => {
            update(
                json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"thinking"}}),
            );
            text("hel");
            text("lo");
            update(
                json!({"sessionUpdate":"tool_call","toolCallId":"t1","title":"Run ls","kind":"execute","status":"pending","rawInput":{"command":"ls"}}),
            );
            update(
                json!({"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"ok"}}]}),
            );
        }
        "ask" => {
            send(
                json!({"jsonrpc":"2.0","id":100,"method":"session/request_permission","params":{
                "sessionId":"fake-1",
                "toolCall":{"toolCallId":"t2","title":"touch a.txt","kind":"execute","rawInput":{"command":"touch a.txt"}},
                "options":[
                    {"optionId":"allow","name":"Allow","kind":"allow_once"},
                    {"optionId":"reject","name":"Reject","kind":"reject_once"}]}}),
            );
            loop {
                let Some(m) = next(rx, Duration::from_secs(30)) else {
                    std::process::exit(1)
                };
                if m["id"] == 100 {
                    let o = &m["result"]["outcome"];
                    match o["outcome"].as_str() {
                        Some("selected") => {
                            let id = o["optionId"].as_str().unwrap();
                            let verdict = if id.starts_with("reject") {
                                "rejected"
                            } else {
                                "allowed"
                            };
                            text(&format!("{verdict}:{id}"))
                        }
                        _ => text("denied"),
                    }
                    break;
                }
                if m["method"] == "session/cancel" {
                    stop = "cancelled";
                    break;
                }
            }
        }
        "slow" => {
            let mut n = 0;
            loop {
                n += 1;
                text(&format!("chunk{n} "));
                if let Some(m) = next(rx, Duration::from_millis(20))
                    && m["method"] == "session/cancel"
                {
                    stop = "cancelled";
                    break;
                }
                if n > 1500 {
                    break;
                }
            }
        }
        "die" => {
            eprintln!("boom");
            std::process::exit(3);
        }
        other => text(&format!("echo:{other}")),
    }
    reply(id, json!({"stopReason": stop}));
}

fn main() {
    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if let Ok(v) = serde_json::from_str(&line)
                && tx.send(v).is_err()
            {
                break;
            }
        }
    });
    while let Some(m) = next(&rx, Duration::from_secs(60)) {
        let id = m["id"].clone();
        match m["method"].as_str() {
            Some("initialize") => reply(
                &id,
                json!({"protocolVersion":1,"agentCapabilities":{"loadSession":true}}),
            ),
            Some("session/new") => {
                reply(&id, json!({"sessionId":"fake-1"}));
                update(
                    json!({"sessionUpdate":"available_commands_update","availableCommands":[{"name":"compact","description":"d"},{"name":"init","description":"i"}]}),
                );
            }
            Some("session/load") => {
                if m["params"]["sessionId"] == "fake-1" {
                    text("old1");
                    text("old2");
                    reply(&id, json!({}));
                } else {
                    send(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32002,"message":"Resource not found"}}),
                    );
                }
            }
            Some("session/prompt") => {
                let p = m["params"]["prompt"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                turn(&id, &p, &rx);
            }
            _ => {}
        }
    }
}
