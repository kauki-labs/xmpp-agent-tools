//! The session loop end to end: the real ACP session talking to the scripted fake agent, and a fake chat.

mod support;

use std::time::Duration;

use support::{ALICE, BOB, CAROL, Rig, settings};
use xmpp_agent::log::LogEvent;
use xmpp_agent::session::SessionError;
use xmpp_agent_xmpp::Event;

#[tokio::test]
async fn a_prompt_should_get_one_answer_message_and_a_tool_line() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "hello").await;
    rig.chat.wait_for(ALICE, "hello").await;
    rig.chat.wait_for_state(ALICE, "active").await;
    let bodies = rig.chat.bodies(ALICE);
    assert_eq!(bodies.len(), 2, "{bodies:?}");
    assert!(
        bodies[0].starts_with("tool: ") && bodies[0].contains("ls"),
        "{bodies:?}"
    );
    assert_eq!(
        bodies[1], "hello",
        "the two text chunks arrive as one message"
    );
    assert_eq!(rig.chat.states(ALICE), ["composing", "active"]);
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_prompt_should_never_show_tool_output_or_thoughts() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "hello").await;
    rig.chat.wait_for_state(ALICE, "active").await;
    let all = rig.chat.bodies(ALICE).join("\n");
    assert!(!all.contains("thinking"), "{all}");
    assert!(!all.contains("ok"), "tool output leaked: {all}");
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_message_from_a_full_jid_in_another_case_should_be_accepted() {
    let rig = Rig::start(settings()).await;
    rig.say("Alice@Chat.Example.org/phone", "hello").await;
    rig.chat.wait_for(ALICE, "hello").await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_message_from_a_stranger_should_get_no_reply_and_a_log_line() {
    let rig = Rig::start(settings()).await;
    rig.say(BOB, "hello").await;
    rig.say(BOB, "/new").await;
    rig.say(BOB, "1").await;
    rig.chat.assert_quiet().await;
    rig.say(ALICE, "hello").await;
    rig.chat.wait_for(ALICE, "hello").await;
    let log = rig.log.events();
    assert_eq!(
        log.iter()
            .filter(|e| matches!(e, LogEvent::IgnoredSender { from } if from == BOB))
            .count(),
        3,
        "{log:?}"
    );
    assert!(rig.chat.bodies(BOB).is_empty());
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_correction_should_be_ignored() {
    let rig = Rig::start(settings()).await;
    rig.event(Event::Chat {
        from: ALICE.to_owned(),
        id: Some("2".to_owned()),
        body: "hello".to_owned(),
        replaces: Some("1".to_owned()),
    })
    .await;
    rig.chat.assert_quiet().await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_permission_should_be_asked_in_chat_and_allowed_when_the_approver_answers_1() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "ask").await;
    let ask = rig.chat.wait_for(ALICE, "Approval #").await;
    assert!(
        ask.contains("touch a.txt") || ask.contains("Reply 1"),
        "{ask}"
    );
    rig.say(ALICE, "1").await;
    rig.chat.wait_for(ALICE, "allowed:allow").await;
    rig.chat.wait_for_state(ALICE, "active").await;
    let log = rig.log.events();
    assert!(
        log.iter()
            .any(|e| matches!(e, LogEvent::ApprovalAsked { tool, .. } if tool == "touch a.txt")),
        "{log:?}"
    );
    assert!(log.iter().any(|e| matches!(e, LogEvent::ApprovalDecided { behavior, reason, .. } if behavior == "allow" && reason == "approved")), "{log:?}");
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_permission_should_be_denied_when_the_approver_answers_3() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "ask").await;
    rig.chat.wait_for(ALICE, "Approval #").await;
    rig.say(ALICE, "3").await;
    rig.chat.wait_for(ALICE, "rejected:reject").await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_permission_should_be_denied_and_the_note_passed_on_when_the_approver_answers_4_with_a_note()
 {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "ask").await;
    rig.chat.wait_for(ALICE, "Approval #").await;
    rig.say(ALICE, "4 too risky for now").await;
    rig.chat.wait_for(ALICE, "rejected:reject").await;
    let follow_up = rig.chat.wait_for(ALICE, "too risky for now").await;
    assert!(follow_up.contains("touch a.txt"), "{follow_up}");
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_permission_should_be_denied_and_the_agent_told_when_nobody_answers_in_time() {
    let mut s = settings();
    s.approval_timeout = Duration::from_secs(1);
    let rig = Rig::start(s).await;
    rig.say(ALICE, "ask").await;
    rig.chat.wait_for(ALICE, "Approval #").await;
    rig.chat.wait_for(ALICE, "rejected:reject").await;
    rig.chat.wait_for(ALICE, "timed out").await;
    let follow_up = rig.chat.wait_for(ALICE, "independent").await;
    assert!(
        follow_up.contains("human") && follow_up.contains("touch a.txt"),
        "{follow_up}"
    );
    let log = rig.log.events();
    assert!(log.iter().any(|e| matches!(e, LogEvent::ApprovalDecided { behavior, reason, .. } if behavior == "deny" && reason == "timeout")), "{log:?}");
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_late_answer_should_not_decide_a_request_that_timed_out() {
    let mut s = settings();
    s.approval_timeout = Duration::from_secs(1);
    let rig = Rig::start(s).await;
    rig.say(ALICE, "ask").await;
    rig.chat.wait_for(ALICE, "timed out").await;
    rig.chat.wait_for(ALICE, "independent").await;
    rig.say(ALICE, "1").await;
    rig.chat.wait_for(ALICE, "echo:1").await;
    assert!(
        !rig.chat
            .bodies(ALICE)
            .iter()
            .any(|b| b.contains("allowed:"))
    );
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn an_allowed_human_who_is_not_an_approver_should_not_decide_a_request() {
    let mut s = settings();
    s.allowed = vec![ALICE.to_owned(), CAROL.to_owned()];
    let rig = Rig::start(s).await;
    rig.say(CAROL, "ask").await;
    rig.chat.wait_for(ALICE, "Approval #").await;
    rig.say(CAROL, "1").await;
    rig.chat.assert_quiet().await;
    assert!(
        !rig.chat
            .bodies(CAROL)
            .iter()
            .any(|b| b.contains("allowed:")),
        "{:?}",
        rig.chat.bodies(CAROL)
    );
    rig.say(ALICE, "3").await;
    rig.chat.wait_for(CAROL, "rejected:reject").await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn stop_should_cancel_the_turn_and_say_so() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "slow").await;
    rig.chat.wait_for_state(ALICE, "composing").await;
    rig.say(ALICE, "/stop").await;
    rig.chat.wait_for(ALICE, "Stopped.").await;
    rig.chat.wait_for_state(ALICE, "active").await;
    rig.say(ALICE, "hello").await;
    rig.chat.wait_for(ALICE, "hello").await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn an_agent_that_dies_should_be_reported_once_and_prompts_should_wait_for_new() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "die").await;
    let notice = rig.chat.wait_for(ALICE, "/new").await;
    assert!(notice.contains("agent"), "{notice}");
    rig.say(ALICE, "hello").await;
    rig.say(ALICE, "hello again").await;
    rig.chat.assert_quiet().await;
    assert_eq!(
        rig.chat
            .bodies(ALICE)
            .iter()
            .filter(|b| b.contains("/new"))
            .count(),
        1,
        "{:?}",
        rig.chat.bodies(ALICE)
    );
    assert!(
        rig.log
            .events()
            .iter()
            .any(|e| matches!(e, LogEvent::AgentExited { .. }))
    );
    rig.say(ALICE, "/new").await;
    rig.chat.wait_for(ALICE, "New session").await;
    rig.say(ALICE, "hello").await;
    rig.chat.wait_for(ALICE, "hello").await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn new_should_start_a_fresh_session_and_deny_what_waited() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "ask").await;
    rig.chat.wait_for(ALICE, "Approval #").await;
    rig.say(ALICE, "/new").await;
    rig.chat.wait_for(ALICE, "New session").await;
    let log = rig.log.events();
    assert!(log.iter().any(|e| matches!(e, LogEvent::ApprovalDecided { behavior, reason, .. } if behavior == "deny" && reason == "shutdown")), "{log:?}");
    rig.say(ALICE, "1").await;
    rig.chat.wait_for(ALICE, "echo:1").await;
    rig.say(ALICE, "hello").await;
    rig.chat.wait_for(ALICE, "hello").await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn status_and_help_should_answer_locally_without_a_turn() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "/status").await;
    let status = rig.chat.wait_for(ALICE, "running").await;
    assert!(
        status.contains("fake-1") && status.contains("idle"),
        "{status}"
    );
    rig.say(ALICE, "/help").await;
    rig.chat.wait_for(ALICE, "/stop").await;
    assert!(
        rig.chat.states(ALICE).is_empty(),
        "no turn ran: {:?}",
        rig.chat.states(ALICE)
    );
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_harness_command_should_go_to_the_agent_as_text() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "/compact now").await;
    rig.chat.wait_for(ALICE, "echo:/compact now").await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_rejected_command_should_get_a_one_line_reply() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "/frobnicate").await;
    let unknown = rig.chat.wait_for(ALICE, "/frobnicate").await;
    assert!(!unknown.contains('\n'), "{unknown}");
    rig.say(ALICE, "/mode plan").await;
    rig.chat.wait_for(ALICE, "/mode").await;
    rig.say(ALICE, "/verbose").await;
    rig.chat.wait_for(ALICE, "/verbose").await;
    assert!(rig.chat.states(ALICE).is_empty());
    rig.say(ALICE, "   ").await;
    rig.chat.assert_quiet().await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn a_reconnect_should_resume_silently() {
    let rig = Rig::start(settings()).await;
    rig.event(Event::Reconnecting).await;
    rig.event(Event::Reconnected).await;
    rig.event(Event::Presence {
        from: ALICE.to_owned(),
        available: true,
    })
    .await;
    rig.event(Event::ChatState {
        from: ALICE.to_owned(),
        state: "composing".to_owned(),
    })
    .await;
    rig.chat.assert_quiet().await;
    rig.say(ALICE, "hello").await;
    rig.chat.wait_for(ALICE, "hello").await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn the_loop_should_end_with_an_error_when_the_connection_ends_for_good() {
    let rig = Rig::start(settings()).await;
    rig.event(Event::Disconnected("stream closed".to_owned()))
        .await;
    let outcome = rig.ended().await;
    assert_eq!(
        outcome,
        Err(SessionError::Disconnected("stream closed".to_owned()))
    );
}

#[tokio::test]
async fn a_group_message_should_be_ignored() {
    let rig = Rig::start(settings()).await;
    rig.event(Event::Groupchat {
        room: "lab@muc.example.org".to_owned(),
        nick: "alice".to_owned(),
        real_jid: Some(ALICE.to_owned()),
        id: None,
        body: "hello".to_owned(),
        replaces: None,
        from_self: false,
    })
    .await;
    rig.chat.assert_quiet().await;
    rig.finish().await.expect("clean end");
}

#[tokio::test]
async fn the_log_should_hold_starts_and_stops_and_no_message_body() {
    let rig = Rig::start(settings()).await;
    rig.say(ALICE, "hello").await;
    rig.chat.wait_for_state(ALICE, "active").await;
    let log = rig.log.clone();
    rig.finish().await.expect("clean end");
    let events = log.events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, LogEvent::AgentStarted { session_id } if session_id == "fake-1")),
        "{events:?}"
    );
    assert!(events.contains(&LogEvent::Stopped), "{events:?}");
    assert!(events.contains(&LogEvent::Started), "{events:?}");
    assert!(!format!("{events:?}").contains("hello"), "{events:?}");
}
