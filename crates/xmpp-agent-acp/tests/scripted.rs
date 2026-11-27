//! Integration tests against `fake-acp-agent`, a scripted ACP agent (see `tests/support/fake_agent.rs`).

use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;
use xmpp_agent_acp::{AcpSession, AgentEvent, Error, SpawnConfig};

fn cfg() -> SpawnConfig {
    SpawnConfig {
        command: env!("CARGO_BIN_EXE_fake-acp-agent").into(),
        args: vec![],
        env: vec![],
        cwd: std::env::temp_dir(),
    }
}

async fn ev(rx: &mut Receiver<AgentEvent>) -> AgentEvent {
    timeout(Duration::from_secs(20), rx.recv())
        .await
        .expect("timed out waiting for an event")
        .expect("channel closed")
}

/// Events up to and including the next `Done`.
async fn turn(rx: &mut Receiver<AgentEvent>) -> Vec<AgentEvent> {
    let mut out = vec![];
    loop {
        let e = ev(rx).await;
        let done = matches!(e, AgentEvent::Done { .. });
        out.push(e);
        if done {
            return out;
        }
    }
}

fn done(reason: &str) -> AgentEvent {
    AgentEvent::Done {
        stop_reason: reason.into(),
    }
}

#[tokio::test]
async fn spawn_opens_a_session_and_reports_commands() {
    let (s, mut rx) = AcpSession::spawn(cfg()).await.unwrap();
    assert_eq!(s.session_id(), "fake-1");
    assert_eq!(
        ev(&mut rx).await,
        AgentEvent::Commands(vec!["compact".into(), "init".into()])
    );
    s.shutdown().await;
}

#[tokio::test]
async fn a_turn_streams_events_and_ends_with_done() {
    let (s, mut rx) = AcpSession::spawn(cfg()).await.unwrap();
    assert!(matches!(ev(&mut rx).await, AgentEvent::Commands(_)));
    s.prompt("hello").await.unwrap();
    let events = turn(&mut rx).await;
    assert_eq!(
        events,
        vec![
            AgentEvent::Thought("thinking".into()),
            AgentEvent::Text("hel".into()),
            AgentEvent::Text("lo".into()),
            AgentEvent::ToolCall {
                id: "t1".into(),
                title: "Run ls".into(),
                kind: "execute".into(),
                raw_input: serde_json::json!({"command": "ls"})
            },
            AgentEvent::ToolUpdate {
                id: "t1".into(),
                status: "completed".into(),
                content: Some("ok".into())
            },
            done("end_turn"),
        ]
    );
    s.shutdown().await;
}

#[tokio::test]
async fn prompts_sent_back_to_back_both_complete_in_order() {
    let (s, mut rx) = AcpSession::spawn(cfg()).await.unwrap();
    s.prompt("hello").await.unwrap();
    s.prompt("second").await.unwrap();
    let first = turn(&mut rx).await;
    let second = turn(&mut rx).await;
    assert!(first.contains(&AgentEvent::Text("hel".into())));
    assert_eq!(
        second,
        vec![AgentEvent::Text("echo:second".into()), done("end_turn")]
    );
    s.shutdown().await;
}

async fn permission(rx: &mut Receiver<AgentEvent>) -> u64 {
    loop {
        if let AgentEvent::Permission {
            request_id,
            tool,
            options,
        } = ev(rx).await
        {
            assert_eq!(tool, "touch a.txt");
            assert_eq!(options.len(), 2);
            assert_eq!(options[0].id, "allow");
            assert_eq!(options[0].kind, "allow_once");
            assert_eq!(options[1].kind, "reject_once");
            return request_id;
        }
    }
}

#[tokio::test]
async fn a_permission_is_allowed() {
    let (s, mut rx) = AcpSession::spawn(cfg()).await.unwrap();
    s.prompt("ask").await.unwrap();
    let id = permission(&mut rx).await;
    s.respond_permission(id, Some("allow")).await.unwrap();
    assert_eq!(
        turn(&mut rx).await,
        vec![AgentEvent::Text("allowed:allow".into()), done("end_turn")]
    );
    s.shutdown().await;
}

#[tokio::test]
async fn a_permission_is_denied() {
    let (s, mut rx) = AcpSession::spawn(cfg()).await.unwrap();
    s.prompt("ask").await.unwrap();
    let id = permission(&mut rx).await;
    s.respond_permission(id, Some("reject")).await.unwrap();
    assert_eq!(
        turn(&mut rx).await,
        vec![AgentEvent::Text("rejected:reject".into()), done("end_turn")]
    );
    s.shutdown().await;
}

#[tokio::test]
async fn a_cancelled_permission_outcome_reaches_the_agent() {
    let (s, mut rx) = AcpSession::spawn(cfg()).await.unwrap();
    s.prompt("ask").await.unwrap();
    let id = permission(&mut rx).await;
    s.respond_permission(id, None).await.unwrap();
    assert_eq!(
        turn(&mut rx).await,
        vec![AgentEvent::Text("denied".into()), done("end_turn")]
    );
    s.shutdown().await;
}

#[tokio::test]
async fn cancel_ends_a_running_turn_and_the_session_stays_usable() {
    let (s, mut rx) = AcpSession::spawn(cfg()).await.unwrap();
    s.prompt("slow").await.unwrap();
    loop {
        if matches!(ev(&mut rx).await, AgentEvent::Text(_)) {
            break;
        }
    }
    s.cancel().await.unwrap();
    let events = turn(&mut rx).await;
    assert_eq!(events.last(), Some(&done("cancelled")));
    s.prompt("again").await.unwrap();
    let again = turn(&mut rx).await;
    assert!(again.contains(&AgentEvent::Text("echo:again".into())));
    s.shutdown().await;
}

#[tokio::test]
async fn cancel_while_waiting_for_a_permission_ends_the_turn() {
    let (s, mut rx) = AcpSession::spawn(cfg()).await.unwrap();
    s.prompt("ask").await.unwrap();
    permission(&mut rx).await;
    s.cancel().await.unwrap();
    assert_eq!(turn(&mut rx).await, vec![done("cancelled")]);
    s.shutdown().await;
}

#[tokio::test]
async fn load_resumes_a_session_without_replaying_history() {
    let (s, mut rx) = AcpSession::load(cfg(), "fake-1").await.unwrap();
    assert_eq!(s.session_id(), "fake-1");
    s.prompt("hello").await.unwrap();
    let events = turn(&mut rx).await;
    assert!(
        !events.contains(&AgentEvent::Text("old1".into())),
        "history replayed: {events:?}"
    );
    assert_eq!(
        events.first(),
        Some(&AgentEvent::Thought("thinking".into()))
    );
    s.shutdown().await;
}

#[tokio::test]
async fn load_of_an_unknown_session_is_an_rpc_error() {
    match AcpSession::load(cfg(), "nope").await {
        Err(Error::Rpc { code, message }) => {
            assert_eq!(code, -32002);
            assert_eq!(message, "Resource not found");
        }
        other => panic!("expected an rpc error, got {:?}", other.map(|_| ())),
    }
}

#[tokio::test]
async fn an_agent_that_exits_yields_exited_with_its_stderr() {
    let (s, mut rx) = AcpSession::spawn(cfg()).await.unwrap();
    s.prompt("die").await.unwrap();
    loop {
        match ev(&mut rx).await {
            AgentEvent::Exited(why) => {
                assert!(why.contains('3'), "exit status missing: {why}");
                assert!(why.contains("boom"), "stderr missing: {why}");
                break;
            }
            AgentEvent::Commands(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(matches!(
        s.prompt("hello").await,
        Err(Error::Exited | Error::Io(_))
    ));
}

#[tokio::test]
async fn spawning_a_missing_command_fails() {
    let mut c = cfg();
    c.command = "/nonexistent/xat-agent".into();
    assert!(matches!(AcpSession::spawn(c).await, Err(Error::Io(_))));
}

#[tokio::test]
async fn an_agent_that_dies_during_the_handshake_is_an_error() {
    let mut c = cfg();
    c.command = "true".into();
    assert!(matches!(AcpSession::spawn(c).await, Err(Error::Exited)));
}

#[tokio::test]
async fn the_env_reaches_the_agent() {
    let mut c = cfg();
    c.command = "sh".into();
    c.args = vec![
        "-c".into(),
        "test \"$XAT_PROBE\" = yes && exec \"$0\"".into(),
        env!("CARGO_BIN_EXE_fake-acp-agent").into(),
    ];
    c.env = vec![("XAT_PROBE".into(), "yes".into())];
    let (s, _rx) = AcpSession::spawn(c).await.unwrap();
    s.shutdown().await;
}
