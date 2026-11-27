//! Opt-in contract test against a real ACP agent. Set `XAT_TEST_ACP_CMD` to its command line (for example `claude-agent-acp`);
//! without it the test returns at once. A missing login is reported as a skip, not a failure.

use std::time::Duration;

use tokio::time::timeout;
use xmpp_agent_acp::{AcpSession, AgentEvent, Error, SpawnConfig};

fn needs_login(text: &str) -> bool {
    let t = text.to_lowercase();
    ["auth", "login", "log in", "credential", "api key"]
        .iter()
        .any(|w| t.contains(w))
}

#[tokio::test]
async fn real_agent_answers_one_prompt() {
    let Ok(cmdline) = std::env::var("XAT_TEST_ACP_CMD") else {
        eprintln!("skipped: XAT_TEST_ACP_CMD is not set");
        return;
    };
    let mut words = cmdline.split_whitespace().map(String::from);
    let command = words.next().expect("XAT_TEST_ACP_CMD is empty");
    let cwd = std::env::temp_dir().join(format!("xat-acp-real-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).unwrap();
    let cfg = SpawnConfig {
        command,
        args: words.collect(),
        env: vec![(
            "CLAUDE_CODE_ENTRYPOINT".into(),
            "xmpp-agent-acp-test".into(),
        )],
        cwd: cwd.clone(),
    };
    let (s, mut rx) = match AcpSession::spawn(cfg).await {
        Ok(x) => x,
        Err(Error::Rpc { message, .. }) if needs_login(&message) => {
            eprintln!("skipped: the agent needs a login: {message}");
            return;
        }
        Err(e) => panic!("spawn failed: {e}"),
    };
    assert!(!s.session_id().is_empty());
    s.prompt("Reply with only the word: ok").await.unwrap();
    let mut text = String::new();
    let outcome = timeout(Duration::from_secs(180), async {
        loop {
            match rx.recv().await {
                Some(AgentEvent::Text(t)) => text.push_str(&t),
                Some(AgentEvent::Permission { request_id, .. }) => {
                    s.respond_permission(request_id, None).await.unwrap();
                }
                Some(AgentEvent::Done { stop_reason }) => break Ok(stop_reason),
                Some(AgentEvent::Exited(why)) => break Err(why),
                Some(_) => {}
                None => break Err("channel closed".into()),
            }
        }
    })
    .await
    .expect("no Done within 180 s");
    s.shutdown().await;
    let _ = std::fs::remove_dir_all(&cwd);
    match outcome {
        Ok(reason) if reason.starts_with("error") && needs_login(&reason) => {
            eprintln!("skipped: the agent needs a login: {reason}");
        }
        Ok(reason) => {
            assert_eq!(reason, "end_turn", "text so far: {text:?}");
            assert!(!text.trim().is_empty(), "no text came back");
        }
        Err(why) if needs_login(&why) => eprintln!("skipped: the agent needs a login: {why}"),
        Err(why) => panic!("agent exited: {why}"),
    }
}
