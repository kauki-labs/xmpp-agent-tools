//! Port of `tests/test_approvals.py`, test for test, then the cases where Python and Rust differ on purpose.
//! Each `py:` comment names the Python test. Expected values of the second part come from `python3` (3.13).

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::*;

const ME: &str = "me@chat.example";
const CTX: &str = "engineering#t1";

#[derive(Clone)]
struct Clock(Arc<Mutex<f64>>);

impl Clock {
    fn now(&self) -> f64 {
        *self.0.lock().unwrap()
    }

    fn advance(&self, seconds: f64) {
        *self.0.lock().unwrap() += seconds;
    }
}

/// Ids `a1`, `a2`, ... and a clock at 1000.
fn make_with(approvers: &[&str], timeout: f64, max_pending: usize) -> (Broker, Clock) {
    let clock = Clock(Arc::new(Mutex::new(1000.0)));
    let reader = clock.clone();
    let mut counter = 0;
    let ids = move || {
        counter += 1;
        format!("a{counter}")
    };
    let broker = Broker::new(
        approvers.iter().copied(),
        Box::new(move || reader.now()),
        Box::new(ids),
        timeout,
        max_pending,
    );
    (broker, clock)
}

fn make() -> (Broker, Clock) {
    make_with(&[ME], 900.0, 20)
}

fn bash_in(broker: &mut Broker, command: &str, ctx: &str, tool_use_id: &str) -> Submitted {
    broker
        .submit(ctx, "Bash", &json!({"command": command}), tool_use_id)
        .unwrap()
}

fn bash(broker: &mut Broker) -> Submitted {
    bash_in(broker, "touch probe.txt", CTX, "tu1")
}

fn ids_of(broker: &Broker) -> Vec<String> {
    broker
        .pending(None)
        .iter()
        .map(|r| r.request_id.clone())
        .collect()
}

fn question() -> Value {
    json!({"questions": [{"question": "Which color?", "header": "Color", "multiSelect": false,
        "options": [{"label": "Red", "description": "warm"}, {"label": "Blue", "description": "cool"}]}]})
}

fn multi_question() -> Value {
    json!({"questions": [{"question": "Which?", "header": "H", "multiSelect": true,
        "options": [{"label": "A", "description": ""}, {"label": "B", "description": ""}, {"label": "C", "description": ""}]}]})
}

mod submit {
    use super::*;

    #[test]
    fn broker_should_show_the_id_the_redacted_call_and_the_choices_when_a_call_is_submitted() {
        // py: Submit.test_message_has_the_id_the_redacted_call_and_the_choices
        let (mut broker, _) = make();
        let sub = broker
            .submit(
                CTX,
                "Bash",
                &json!({"command": "DB_PASSWORD=hunter2 psql -h db"}),
                "tu1",
            )
            .unwrap();
        assert!(sub.decision.is_none());
        assert_eq!(sub.request.unwrap().request_id, "a1");
        let text = sub.text.unwrap();
        assert!(text.contains("#a1"));
        assert!(text.contains("psql -h db"));
        assert!(!text.contains("hunter2"));
        for choice in ["1", "2", "3"] {
            assert!(text.contains(choice), "{choice}");
        }
    }

    #[test]
    fn broker_should_give_the_request_a_deadline_when_a_call_is_submitted() {
        // py: Submit.test_request_carries_its_deadline
        let (mut broker, clock) = make_with(&[ME], 600.0, 20);
        assert_eq!(
            bash(&mut broker).request.unwrap().deadline,
            clock.now() + 600.0
        );
    }
}

mod replies {
    use super::*;

    #[test]
    fn broker_should_allow_once_and_map_to_the_permission_result_when_the_approver_says_one() {
        // py: Replies.test_one_allows_once_and_maps_to_the_permission_tool_result
        let (mut broker, _) = make();
        bash(&mut broker);
        let rep = broker.handle_reply(ME, CTX, "1");
        assert_eq!(rep.kind, ReplyKind::Decided);
        let decision = rep.decision.unwrap();
        assert_eq!(decision.behavior, Behavior::Allow);
        assert_eq!(
            decision.permission_result(),
            json!({"behavior": "allow", "updatedInput": {"command": "touch probe.txt"}})
        );
        assert!(broker.pending(None).is_empty());
    }

    #[test]
    fn broker_should_treat_words_like_numbers_when_the_approver_answers_with_a_word() {
        // py: Replies.test_words_work_like_numbers
        for (word, behavior) in [
            ("y", Behavior::Allow),
            ("yes", Behavior::Allow),
            ("allow", Behavior::Allow),
            ("n", Behavior::Deny),
            ("no", Behavior::Deny),
            ("deny", Behavior::Deny),
            ("3", Behavior::Deny),
        ] {
            let (mut broker, _) = make();
            bash(&mut broker);
            assert_eq!(
                broker
                    .handle_reply(ME, CTX, word)
                    .decision
                    .unwrap()
                    .behavior,
                behavior,
                "{word}"
            );
        }
    }

    #[test]
    fn broker_should_give_the_model_a_message_when_the_approver_denies() {
        // py: Replies.test_deny_gives_the_model_a_message
        let (mut broker, _) = make();
        bash(&mut broker);
        let res = broker
            .handle_reply(ME, CTX, "3")
            .decision
            .unwrap()
            .permission_result();
        assert_eq!(
            res,
            json!({"behavior": "deny", "message": "Denied by the operator via chat."})
        );
    }

    #[test]
    fn broker_should_deny_with_the_note_when_the_approver_says_four_and_a_note() {
        // py: Replies.test_four_denies_with_a_note
        let (mut broker, _) = make();
        bash(&mut broker);
        let res = broker
            .handle_reply(ME, CTX, "4 too risky, use a temp dir")
            .decision
            .unwrap()
            .permission_result();
        assert_eq!(res["behavior"], "deny");
        assert!(
            res["message"]
                .as_str()
                .unwrap()
                .contains("too risky, use a temp dir")
        );
    }

    #[test]
    fn broker_should_refuse_a_stranger_when_the_stranger_answers() {
        // py: Replies.test_a_stranger_cannot_decide
        let (mut broker, _) = make();
        bash(&mut broker);
        let rep = broker.handle_reply("mallory@chat.example", CTX, "1");
        assert_eq!(rep.kind, ReplyKind::Forbidden);
        assert!(rep.decision.is_none());
        assert_eq!(broker.pending(None).len(), 1);
    }

    #[test]
    fn broker_should_let_nobody_decide_when_there_are_no_approvers() {
        // py: Replies.test_no_approvers_means_nobody_decides
        let (mut broker, _) = make_with(&[], 900.0, 20);
        bash(&mut broker);
        assert_eq!(broker.handle_reply(ME, CTX, "1").kind, ReplyKind::Forbidden);
    }

    #[test]
    fn broker_should_ignore_the_resource_and_the_case_when_it_compares_a_jid() {
        // py: Replies.test_the_resource_and_the_case_of_the_jid_do_not_matter
        let (mut broker, _) = make();
        bash(&mut broker);
        assert_eq!(
            broker.handle_reply("ME@Chat.Example/phone", CTX, "1").kind,
            ReplyKind::Decided
        );
    }

    #[test]
    fn broker_should_not_approve_when_the_answer_is_a_polite_yes() {
        // py: Replies.test_a_polite_yes_is_not_an_approval
        for text in [
            "yes please",
            "ok but check the path first",
            "1 please",
            "allow it if the tests pass",
        ] {
            let (mut broker, _) = make();
            bash(&mut broker);
            assert_eq!(
                broker.handle_reply(ME, CTX, text).kind,
                ReplyKind::NotAReply,
                "{text}"
            );
            assert_eq!(broker.pending(None).len(), 1);
        }
    }

    #[test]
    fn broker_should_refuse_a_nickname_when_the_sender_is_a_muc_occupant() {
        // py: Replies.test_a_nickname_is_not_a_jid
        let (mut broker, _) = make();
        bash(&mut broker);
        assert_eq!(
            broker
                .handle_reply("engineering@rooms.chat.example/me", CTX, "1")
                .kind,
            ReplyKind::Forbidden
        );
    }

    #[test]
    fn broker_should_not_reply_when_the_approver_chats() {
        // py: Replies.test_ordinary_chat_from_the_approver_is_not_a_reply
        let (mut broker, _) = make();
        bash(&mut broker);
        assert_eq!(
            broker.handle_reply(ME, CTX, "why does it need that?").kind,
            ReplyKind::NotAReply
        );
        assert_eq!(broker.pending(None).len(), 1);
    }

    #[test]
    fn broker_should_not_flag_a_stranger_when_the_stranger_chats() {
        // py: Replies.test_ordinary_chat_from_a_stranger_is_not_flagged
        let (mut broker, _) = make();
        bash(&mut broker);
        assert_eq!(
            broker
                .handle_reply("mallory@chat.example", CTX, "lunch?")
                .kind,
            ReplyKind::NotAReply
        );
    }

    #[test]
    fn broker_should_not_reply_when_nothing_is_pending() {
        // py: Replies.test_nothing_pending_means_not_a_reply
        let (mut broker, _) = make();
        assert_eq!(broker.handle_reply(ME, CTX, "1").kind, ReplyKind::NotAReply);
    }

    #[test]
    fn broker_should_isolate_contexts_when_a_reply_comes_from_another_one() {
        // py: Replies.test_contexts_are_isolated
        let (mut broker, _) = make();
        bash_in(&mut broker, "touch probe.txt", "engineering#t1", "tu1");
        assert_eq!(
            broker.handle_reply(ME, "engineering#t2", "1").kind,
            ReplyKind::NotAReply
        );
        assert_eq!(broker.pending(None).len(), 1);
    }
}

mod ids {
    use super::*;

    #[test]
    fn broker_should_ask_for_an_id_when_two_requests_are_pending() {
        // py: Ids.test_two_pending_need_an_id
        let (mut broker, _) = make();
        bash_in(&mut broker, "touch probe.txt", CTX, "tu1");
        bash_in(&mut broker, "touch probe.txt", CTX, "tu2");
        let rep = broker.handle_reply(ME, CTX, "1");
        assert_eq!(rep.kind, ReplyKind::Ambiguous);
        assert!(rep.notice.contains("#a1"));
        assert!(rep.notice.contains("#a2"));
        assert_eq!(broker.pending(None).len(), 2);
    }

    #[test]
    fn broker_should_accept_the_id_before_or_after_the_answer() {
        // py: Ids.test_the_id_may_come_before_or_after_the_answer
        let (mut broker, _) = make();
        bash_in(&mut broker, "touch probe.txt", CTX, "tu1");
        bash_in(&mut broker, "touch probe.txt", CTX, "tu2");
        assert_eq!(
            broker
                .handle_reply(ME, CTX, "1 #a2")
                .decision
                .unwrap()
                .request
                .request_id,
            "a2"
        );
        assert_eq!(
            broker
                .handle_reply(ME, CTX, "#a1 3")
                .decision
                .unwrap()
                .request
                .request_id,
            "a1"
        );
    }

    #[test]
    fn broker_should_report_an_unknown_id_and_change_nothing() {
        // py: Ids.test_an_unknown_id_is_reported_and_changes_nothing
        let (mut broker, _) = make();
        bash(&mut broker);
        let rep = broker.handle_reply(ME, CTX, "1 #zz");
        assert_eq!(rep.kind, ReplyKind::UnknownRequest);
        assert_eq!(broker.pending(None).len(), 1);
    }

    #[test]
    fn broker_should_decide_a_request_at_most_once() {
        // py: Ids.test_a_request_decides_at_most_once
        let (mut broker, _) = make();
        bash(&mut broker);
        assert_eq!(
            broker.handle_reply(ME, CTX, "1 #a1").kind,
            ReplyKind::Decided
        );
        let again = broker.handle_reply(ME, CTX, "3 #a1");
        assert_eq!(again.kind, ReplyKind::UnknownRequest);
        assert!(again.notice.contains("no longer pending"));
        assert!(again.decision.is_none());
    }

    #[test]
    fn broker_should_keep_an_id_in_its_context_when_another_context_names_it() {
        // py: Ids.test_an_id_belongs_to_its_context
        let (mut broker, _) = make();
        bash_in(&mut broker, "touch probe.txt", "engineering#t1", "tu1");
        assert_eq!(
            broker.handle_reply(ME, "engineering#t2", "1 #a1").kind,
            ReplyKind::UnknownRequest
        );
        assert_eq!(broker.pending(None).len(), 1);
    }
}

mod timeouts {
    use super::*;

    #[test]
    fn broker_should_not_expire_a_request_early() {
        // py: Timeouts.test_nothing_expires_early
        let (mut broker, clock) = make_with(&[ME], 900.0, 20);
        bash(&mut broker);
        clock.advance(899.0);
        assert!(broker.tick().is_empty());
    }

    #[test]
    fn broker_should_deny_once_when_a_request_times_out() {
        // py: Timeouts.test_a_timeout_denies
        let (mut broker, clock) = make_with(&[ME], 900.0, 20);
        bash(&mut broker);
        clock.advance(901.0);
        let decisions = broker.tick();
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].behavior, Behavior::Deny);
        assert_eq!(decisions[0].reason, Reason::Timeout);
        assert!(decisions[0].notice.contains("#a1"));
        assert_eq!(decisions[0].permission_result()["behavior"], "deny");
        assert!(broker.pending(None).is_empty());
        assert!(broker.tick().is_empty()); // once only
    }

    #[test]
    fn broker_should_refuse_a_late_reply_when_the_request_has_expired() {
        // py: Timeouts.test_a_late_reply_is_refused
        let (mut broker, clock) = make_with(&[ME], 900.0, 20);
        bash(&mut broker);
        clock.advance(901.0);
        let rep = broker.handle_reply(ME, CTX, "1"); // lazy expiry: the reply path expires first
        assert_ne!(rep.kind, ReplyKind::Decided);
        assert!(broker.pending(None).is_empty());
    }

    #[test]
    fn broker_should_deny_everything_pending_when_it_closes() {
        // py: Timeouts.test_close_denies_everything_pending
        let (mut broker, _) = make();
        bash_in(&mut broker, "touch probe.txt", CTX, "tu1");
        bash_in(&mut broker, "touch probe.txt", "other#t", "tu2");
        let decisions = broker.close();
        assert!(decisions.iter().all(|d| d.reason == Reason::Shutdown));
        assert!(decisions.iter().all(|d| d.behavior == Behavior::Deny));
        assert_eq!(decisions.len(), 2);
        assert!(broker.pending(None).is_empty());
    }

    #[test]
    fn broker_should_report_what_had_already_expired_when_it_closes() {
        // py: Timeouts.test_close_still_reports_what_had_already_expired
        let (mut broker, clock) = make_with(&[ME], 900.0, 20);
        bash_in(&mut broker, "touch probe.txt", CTX, "tu1");
        clock.advance(901.0);
        broker.handle_reply(ME, CTX, "hello"); // expires lazily, nobody ticked yet
        bash_in(&mut broker, "touch probe.txt", CTX, "tu2");
        let mut reasons: Vec<&str> = broker.close().iter().map(|d| d.reason.as_str()).collect();
        reasons.sort_unstable();
        assert_eq!(reasons, ["shutdown", "timeout"]);
    }
}

mod session {
    use super::*;

    #[test]
    fn broker_should_remember_the_exact_call_when_the_approver_says_two() {
        // py: AllowForSession.test_two_remembers_the_exact_call
        let (mut broker, _) = make();
        bash_in(&mut broker, "make test", CTX, "tu1");
        assert_eq!(
            broker.handle_reply(ME, CTX, "2").decision.unwrap().reason,
            Reason::ApprovedSession
        );
        let again = bash_in(&mut broker, "make test", CTX, "tu2");
        let decision = again.decision.unwrap();
        assert_eq!(decision.behavior, Behavior::Allow);
        assert_eq!(decision.reason, Reason::Remembered);
        assert!(again.text.is_none());
    }

    #[test]
    fn broker_should_still_ask_when_the_call_or_the_context_differs() {
        // py: AllowForSession.test_a_different_call_or_context_still_asks
        let (mut broker, _) = make();
        bash_in(&mut broker, "make test", CTX, "tu1");
        broker.handle_reply(ME, CTX, "2");
        assert!(
            bash_in(&mut broker, "make deploy", CTX, "tu2")
                .decision
                .is_none()
        );
        assert!(
            bash_in(&mut broker, "make test", "other#t", "tu3")
                .decision
                .is_none()
        );
    }

    #[test]
    fn broker_should_clear_the_memory_when_a_context_is_forgotten() {
        // py: AllowForSession.test_forget_clears_the_memory
        let (mut broker, _) = make();
        bash_in(&mut broker, "make test", CTX, "tu1");
        broker.handle_reply(ME, CTX, "2");
        broker.forget(CTX);
        assert!(
            bash_in(&mut broker, "make test", CTX, "tu2")
                .decision
                .is_none()
        );
    }

    #[test]
    fn broker_should_never_remember_a_plan_approval() {
        // py: AllowForSession.test_plan_approval_is_never_remembered
        let (mut broker, _) = make();
        broker
            .submit(CTX, "ExitPlanMode", &json!({"plan": "do things"}), "tu1")
            .unwrap();
        assert_eq!(
            broker.handle_reply(ME, CTX, "2").decision.unwrap().reason,
            Reason::Approved
        );
        assert!(
            broker
                .submit(CTX, "ExitPlanMode", &json!({"plan": "do things"}), "tu2")
                .unwrap()
                .decision
                .is_none()
        );
    }
}

mod id_allocation {
    use super::*;

    fn with_ids(source: &[&str]) -> Broker {
        let mut queue: VecDeque<String> = source.iter().map(|s| (*s).to_owned()).collect();
        let ids = move || queue.pop_front().unwrap();
        Broker::new([ME], Box::new(|| 1000.0), Box::new(ids), 900.0, 20)
    }

    #[test]
    fn broker_should_replace_a_colliding_id() {
        // py: IdAllocation.test_a_colliding_id_is_replaced
        let mut broker = with_ids(&["a1", "a1", "a1", "a2"]);
        let first = bash_in(&mut broker, "touch probe.txt", CTX, "tu1");
        let second = bash_in(&mut broker, "touch probe.txt", CTX, "tu2");
        assert_eq!(first.request.unwrap().request_id, "a1");
        assert_eq!(second.request.unwrap().request_id, "a2");
    }

    #[test]
    fn broker_should_not_reuse_a_resolved_id() {
        // py: IdAllocation.test_a_resolved_id_is_not_reused
        let mut broker = with_ids(&["a1", "a1", "a2"]);
        bash(&mut broker);
        broker.handle_reply(ME, CTX, "1");
        assert_eq!(
            bash_in(&mut broker, "touch probe.txt", CTX, "tu2")
                .request
                .unwrap()
                .request_id,
            "a2"
        );
    }
}

mod limits {
    use super::*;

    #[test]
    fn broker_should_deny_at_once_when_too_many_requests_are_pending() {
        // py: Limits.test_too_many_pending_is_denied_at_once
        let (mut broker, _) = make_with(&[ME], 900.0, 2);
        bash_in(&mut broker, "touch probe.txt", CTX, "tu1");
        bash_in(&mut broker, "touch probe.txt", CTX, "tu2");
        let third = bash_in(&mut broker, "touch probe.txt", CTX, "tu3");
        let decision = third.decision.unwrap();
        assert_eq!(decision.behavior, Behavior::Deny);
        assert_eq!(decision.reason, Reason::TooMany);
        assert_eq!(broker.pending(None).len(), 2);
    }
}

mod questions {
    use super::*;

    #[test]
    fn broker_should_deny_and_not_show_a_malformed_question() {
        // py: Questions.test_malformed_questions_are_denied_not_shown
        for bad in [
            json!({}),
            json!({"questions": []}),
            json!({"questions": "x"}),
            json!({"questions": [{"question": "Q?"}]}),
            json!({"questions": [{"question": "Q?", "options": []}]}),
            json!({"questions": [{"question": "Q?", "options": [{"description": "no label"}]}]}),
        ] {
            let (mut broker, _) = make();
            let sub = broker.submit_question(CTX, &bad, "tu1").unwrap();
            let decision = sub.decision.unwrap();
            assert_eq!(decision.behavior, Behavior::Deny, "{bad}");
            assert_eq!(decision.reason, Reason::Malformed);
            assert!(sub.text.is_none());
            assert!(broker.pending(None).is_empty());
        }
    }

    #[test]
    fn broker_should_call_several_numbers_invalid_when_the_question_is_single_select() {
        // py: Questions.test_several_numbers_on_a_single_select_are_invalid
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        let rep = broker.handle_reply(ME, CTX, "1,2");
        assert_eq!(rep.kind, ReplyKind::Invalid);
        assert_eq!(broker.pending(None).len(), 1);
    }

    #[test]
    fn broker_should_number_the_options_when_it_shows_a_question() {
        // py: Questions.test_options_are_numbered_in_the_message
        let (mut broker, _) = make();
        let text = broker
            .submit_question(CTX, &question(), "tu1")
            .unwrap()
            .text
            .unwrap();
        for part in ["Which color?", "1", "Red", "2", "Blue"] {
            assert!(text.contains(part), "{part}");
        }
    }

    #[test]
    fn broker_should_answer_with_the_label_when_the_approver_sends_a_number() {
        // py: Questions.test_a_number_answers_with_the_label
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        let rep = broker.handle_reply(ME, CTX, "2");
        assert_eq!(rep.kind, ReplyKind::Decided);
        // experiments E4: the answer goes back as updatedInput.answers, keyed by the question text
        let mut expected = question();
        expected["answers"] = json!({"Which color?": "Blue"});
        assert_eq!(
            rep.decision.unwrap().permission_result(),
            json!({"behavior": "allow", "updatedInput": expected})
        );
    }

    #[test]
    fn broker_should_take_free_text_as_an_answer() {
        // py: Questions.test_free_text_is_an_answer_too
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        let rep = broker.handle_reply(ME, CTX, "green, like the logo");
        let updated = rep.decision.unwrap().updated_input.unwrap();
        assert_eq!(
            updated["answers"],
            json!({"Which color?": "green, like the logo"})
        );
    }

    #[test]
    fn broker_should_keep_waiting_when_the_number_is_out_of_range() {
        // py: Questions.test_an_out_of_range_number_is_invalid_and_keeps_waiting
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        for bad in ["9", "0"] {
            assert_eq!(
                broker.handle_reply(ME, CTX, bad).kind,
                ReplyKind::Invalid,
                "{bad}"
            );
            assert_eq!(broker.pending(None).len(), 1);
        }
    }

    #[test]
    fn broker_should_refuse_a_stranger_when_the_stranger_answers_a_question() {
        // py: Questions.test_a_stranger_cannot_answer
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        assert_eq!(
            broker.handle_reply("mallory@chat.example", CTX, "1").kind,
            ReplyKind::Forbidden
        );
    }

    #[test]
    fn broker_should_ask_several_questions_one_after_the_other() {
        // py: Questions.test_several_questions_are_asked_one_after_the_other
        let two = json!({"questions": [
            question()["questions"][0].clone(),
            {"question": "Which size?", "header": "Size", "multiSelect": false, "options": [{"label": "S", "description": ""}, {"label": "L", "description": ""}]},
        ]});
        let (mut broker, _) = make();
        broker.submit_question(CTX, &two, "tu1").unwrap();
        let first = broker.handle_reply(ME, CTX, "1");
        assert_eq!(first.kind, ReplyKind::Next);
        assert!(first.notice.contains("Which size?"));
        let second = broker.handle_reply(ME, CTX, "2");
        assert_eq!(second.kind, ReplyKind::Decided);
        let updated = second.decision.unwrap().updated_input.unwrap();
        assert_eq!(
            updated["answers"],
            json!({"Which color?": "Red", "Which size?": "L"})
        );
    }

    #[test]
    fn broker_should_join_the_labels_when_the_question_is_multi_select() {
        // py: Questions.test_multi_select_joins_the_labels
        // The joined format is a guess: experiments E4 covered single select only. Verify against claude before relying on it.
        let (mut broker, _) = make();
        broker
            .submit_question(CTX, &multi_question(), "tu1")
            .unwrap();
        let updated = broker
            .handle_reply(ME, CTX, "1,3")
            .decision
            .unwrap()
            .updated_input
            .unwrap();
        assert_eq!(updated["answers"], json!({"Which?": "A, C"}));
    }

    #[test]
    fn broker_should_deny_a_question_when_it_times_out() {
        // py: Questions.test_a_question_that_times_out_is_denied
        let (mut broker, clock) = make_with(&[ME], 900.0, 20);
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        clock.advance(901.0);
        assert_eq!(broker.tick()[0].behavior, Behavior::Deny);
    }

    #[test]
    fn broker_should_redact_the_question_when_it_shows_it() {
        // py: Questions.test_questions_are_redacted
        let q = json!({"questions": [{"question": "Use token=abc123 for the call?", "header": "H", "multiSelect": false,
            "options": [{"label": "Yes", "description": ""}, {"label": "No", "description": ""}]}]});
        let (mut broker, _) = make();
        assert!(
            !broker
                .submit_question(CTX, &q, "tu1")
                .unwrap()
                .text
                .unwrap()
                .contains("abc123")
        );
    }
}

/// Cases the Python tests do not cover, where Python and Rust text rules could differ. Expected values are from `python3`.
mod exactness {
    use super::*;

    #[test]
    fn parse_choice_should_match_python_when_the_text_has_punctuation_case_or_odd_whitespace() {
        let cases: [(&str, Option<(Choice, &str)>); 22] = [
            ("1", Some((Choice::Allow, ""))),
            ("Y.", Some((Choice::Allow, ""))),
            ("YES!!", Some((Choice::Allow, ""))),
            ("1.", Some((Choice::Allow, ""))),
            ("OK", Some((Choice::Allow, ""))),
            ("yes.!", Some((Choice::Allow, ""))),
            ("\u{1f}1", Some((Choice::Allow, ""))),
            ("2", Some((Choice::Session, ""))),
            ("2!", Some((Choice::Session, ""))),
            ("4.", Some((Choice::Deny, ""))),
            ("no.", Some((Choice::Deny, ""))),
            ("no thanks", Some((Choice::Deny, "thanks"))),
            ("4 x  y", Some((Choice::Deny, "x  y"))),
            (" 3\u{2003}note here \n", Some((Choice::Deny, "note here"))),
            (".", None),
            (" ", None),
            ("", None),
            ("2 x", None),
            ("1\u{1f}x", None),
            ("\u{130}", None),
            ("yes please", None),
            ("5", None),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_choice(text), expected, "{text:?}");
        }
    }

    #[test]
    fn split_id_should_match_python_when_the_id_is_before_or_after_the_text() {
        let cases: [(&str, (Option<&str>, &str)); 20] = [
            ("#a1 1", (Some("a1"), "1")),
            ("#a1\u{1f}1", (Some("a1"), "1")),
            ("#a1\u{a0}1", (Some("a1"), "1")),
            ("1 #a1", (Some("a1"), "1")),
            ("1\u{1f}#a1", (Some("a1"), "1")),
            ("x\n#a1", (Some("a1"), "x")),
            ("1 #A1 ", (Some("A1"), "1")),
            ("1  #a1", (Some("a1"), "1")),
            ("#a1", (Some("a1"), "")),
            ("#a1\n", (Some("a1"), "")),
            ("#a1 ", (Some("a1"), "")),
            ("#a1 #a2", (Some("a1"), "#a2")),
            ("a #b #c", (Some("c"), "a #b")),
            ("#\u{e9}1 1", (None, "#\u{e9}1 1")),
            ("# a1", (None, "# a1")),
            ("#\u{663} 1", (None, "#\u{663} 1")),
            ("1 #a-b", (None, "1 #a-b")),
            ("1#a", (None, "1#a")),
            ("#a-b 1", (None, "#a-b 1")),
            ("x #", (None, "x #")),
        ];
        for (text, expected) in cases {
            assert_eq!(split_id(text), expected, "{text:?}");
        }
    }

    #[test]
    fn broker_should_compare_a_jid_the_way_python_does_when_it_has_odd_characters() {
        // Python: _bare("MEİ@Chat.Example/phone") == "mei̇@chat.example"; _bare(" \x1fme@x/y") == "me@x"; _bare("me@x /y") == "me@x"
        let (mut broker, _) = make_with(&["mei\u{307}@chat.example"], 900.0, 20);
        bash(&mut broker);
        assert_eq!(
            broker
                .handle_reply("ME\u{130}@Chat.Example/phone", CTX, "1")
                .kind,
            ReplyKind::Decided
        );

        let (mut broker, _) = make_with(&["\u{1f} me@chat.example/laptop"], 900.0, 20);
        bash(&mut broker);
        assert_eq!(
            broker
                .handle_reply("me@chat.example /phone/x", CTX, "1")
                .kind,
            ReplyKind::Decided
        );
    }

    #[test]
    fn broker_should_round_the_minutes_half_to_even_when_it_words_the_timeout() {
        // Python: round(150 / 60) == 2, round(30 / 60) == 0, round(90 / 60) == 2, round(210 / 60) == 4
        for (timeout, minutes) in [(150.0, 2), (30.0, 0), (90.0, 2), (210.0, 4), (900.0, 15)] {
            let (mut broker, clock) = make_with(&[ME], timeout, 20);
            let text = bash(&mut broker).text.unwrap();
            assert!(
                text.ends_with(&format!(
                    "It is denied after {minutes} min without an answer."
                )),
                "{text}"
            );
            clock.advance(timeout);
            let notice = broker.tick().remove(0).notice;
            assert_eq!(
                notice,
                format!("Approval #a1 timed out after {minutes} min and was denied.")
            );
        }
    }

    #[test]
    fn broker_should_word_the_request_exactly_when_it_submits_a_call() {
        let (mut broker, _) = make_with(&[ME], 150.0, 20);
        let text = bash(&mut broker).text.unwrap();
        assert_eq!(
            text,
            "Approval #a1: Bash: touch probe.txt\nReply 1 to allow once, 2 to allow this exact call again in this session, 3 to deny, \
             or 4 and a note to deny with a reason. It is denied after 2 min without an answer."
        );
    }

    #[test]
    fn broker_should_expire_a_request_when_the_clock_reaches_the_deadline() {
        let (mut broker, clock) = make_with(&[ME], 900.0, 20);
        bash(&mut broker);
        clock.advance(900.0);
        assert_eq!(broker.tick().len(), 1);
    }

    #[test]
    fn broker_should_say_how_a_timeout_ended_when_it_denies() {
        let (mut broker, clock) = make();
        bash(&mut broker);
        clock.advance(900.0);
        let decision = broker.tick().remove(0);
        assert_eq!(
            decision.message,
            "Timed out waiting for the operator via chat."
        );
        assert_eq!(
            decision.notice,
            "Approval #a1 timed out after 15 min and was denied."
        );
        assert_eq!(
            decision.permission_result(),
            json!({"behavior": "deny", "message": "Timed out waiting for the operator via chat."})
        );
    }

    #[test]
    fn broker_should_expire_in_the_order_of_submission_when_several_time_out() {
        let (mut broker, clock) = make();
        for tu in ["tu1", "tu2", "tu3"] {
            bash_in(&mut broker, "touch probe.txt", CTX, tu);
        }
        clock.advance(900.0);
        let ids: Vec<String> = broker
            .tick()
            .iter()
            .map(|d| d.request.request_id.clone())
            .collect();
        assert_eq!(ids, ["a1", "a2", "a3"]);
    }

    #[test]
    fn broker_should_hand_over_a_timeout_found_by_a_reply_when_the_next_tick_comes() {
        let (mut broker, clock) = make();
        bash(&mut broker);
        clock.advance(901.0);
        assert_eq!(broker.handle_reply(ME, CTX, "1").kind, ReplyKind::NotAReply);
        let due = broker.tick();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].reason, Reason::Timeout);
        assert!(broker.tick().is_empty());
    }

    #[test]
    fn broker_should_report_due_timeouts_before_the_shutdown_denials_when_it_closes() {
        let (mut broker, clock) = make();
        bash_in(&mut broker, "touch probe.txt", CTX, "tu1");
        clock.advance(901.0);
        broker.handle_reply(ME, CTX, "hello");
        bash_in(&mut broker, "touch probe.txt", CTX, "tu2");
        bash_in(&mut broker, "touch probe.txt", "other#t", "tu3");
        let decisions = broker.close();
        let seen: Vec<(&str, &str)> = decisions
            .iter()
            .map(|d| (d.request.request_id.as_str(), d.reason.as_str()))
            .collect();
        assert_eq!(
            seen,
            [("a1", "timeout"), ("a2", "shutdown"), ("a3", "shutdown")]
        );
        assert_eq!(
            decisions[1].message,
            "The bridge shut down before the operator answered."
        );
        assert_eq!(
            decisions[1].notice,
            "Approval #a2 was denied: the bridge is shutting down."
        );
        assert!(broker.close().is_empty());
    }

    #[test]
    fn broker_should_list_the_requests_of_one_context_when_pending_is_given_it() {
        let (mut broker, _) = make();
        bash_in(&mut broker, "touch probe.txt", CTX, "tu1");
        bash_in(&mut broker, "touch probe.txt", "other#t", "tu2");
        bash_in(&mut broker, "touch probe.txt", CTX, "tu3");
        assert_eq!(ids_of(&broker), ["a1", "a2", "a3"]);
        let in_ctx: Vec<&str> = broker
            .pending(Some(CTX))
            .iter()
            .map(|r| r.request_id.as_str())
            .collect();
        assert_eq!(in_ctx, ["a1", "a3"]);
        assert!(broker.pending(Some("nowhere")).is_empty());
    }

    #[test]
    fn broker_should_say_why_an_id_is_unknown_when_it_was_decided_or_never_pending() {
        let (mut broker, _) = make();
        bash(&mut broker);
        assert_eq!(
            broker.handle_reply(ME, CTX, "1 #zz").notice,
            "Request #zz is not pending here."
        );
        bash_in(&mut broker, "touch probe.txt", "other#t", "tu2");
        assert_eq!(
            broker.handle_reply(ME, CTX, "1 #a2").notice,
            "Request #a2 is not pending here."
        );
        broker.handle_reply(ME, CTX, "1 #a1");
        assert_eq!(
            broker.handle_reply(ME, CTX, "1 #a1").notice,
            "Request #a1 is no longer pending (already decided or timed out)."
        );
    }

    #[test]
    fn broker_should_say_no_longer_pending_when_the_id_timed_out() {
        let (mut broker, clock) = make();
        bash(&mut broker);
        clock.advance(901.0);
        assert_eq!(
            broker.handle_reply(ME, CTX, "1 #a1").kind,
            ReplyKind::UnknownRequest
        );
    }

    #[test]
    fn broker_should_forget_the_oldest_resolved_id_when_256_are_resolved() {
        let (mut broker, _) = make();
        for n in 1..=257 {
            bash_in(&mut broker, "touch probe.txt", CTX, "tu");
            assert_eq!(
                broker.handle_reply(ME, CTX, &format!("1 #a{n}")).kind,
                ReplyKind::Decided
            );
        }
        assert_eq!(
            broker.handle_reply(ME, CTX, "1 #a1").notice,
            "Request #a1 is not pending here."
        );
        assert_eq!(
            broker.handle_reply(ME, CTX, "1 #a2").notice,
            "Request #a2 is no longer pending (already decided or timed out)."
        );
    }

    #[test]
    fn broker_should_not_flag_chat_with_an_id_when_it_is_not_a_decision() {
        let (mut broker, _) = make();
        bash(&mut broker);
        assert_eq!(
            broker
                .handle_reply("mallory@chat.example", CTX, "see #123 for details")
                .kind,
            ReplyKind::NotAReply
        );
        assert_eq!(
            broker.handle_reply(ME, CTX, "#123").kind,
            ReplyKind::NotAReply
        );
        assert_eq!(
            broker
                .handle_reply("mallory@chat.example", CTX, "1 #zz")
                .kind,
            ReplyKind::Forbidden
        );
    }

    #[test]
    fn broker_should_name_the_pending_ids_when_the_answer_is_ambiguous() {
        let (mut broker, _) = make();
        bash_in(&mut broker, "touch probe.txt", CTX, "tu1");
        bash_in(&mut broker, "touch probe.txt", CTX, "tu2");
        assert_eq!(
            broker.handle_reply(ME, CTX, "1").notice,
            "Several requests are waiting (#a1, #a2). Add the id to your answer, for example `1 #a1`."
        );
        assert_eq!(
            broker.handle_reply(ME, CTX, "1,2").kind,
            ReplyKind::Ambiguous
        );
        assert_eq!(
            broker.handle_reply("mallory@chat.example", CTX, "1").kind,
            ReplyKind::Forbidden
        );
        assert_eq!(
            broker.handle_reply(ME, CTX, "what is a2?").kind,
            ReplyKind::NotAReply
        );
    }

    #[test]
    fn broker_should_say_deny_with_the_note_when_a_note_follows_the_denial() {
        let (mut broker, _) = make();
        bash(&mut broker);
        let decision = broker
            .handle_reply(ME, CTX, "4 too risky")
            .decision
            .unwrap();
        assert_eq!(decision.reason, Reason::Denied);
        assert_eq!(
            decision.message,
            "Denied by the operator via chat: too risky"
        );
        assert_eq!(decision.notice, "Denied #a1.");
    }

    #[test]
    fn broker_should_cut_the_note_at_300_characters_when_the_note_is_long() {
        for (unit, expected) in [("x", 300), ("\u{e9}", 300)] {
            let (mut broker, _) = make();
            bash(&mut broker);
            let decision = broker
                .handle_reply(ME, CTX, &format!("4 {}", unit.repeat(400)))
                .decision
                .unwrap();
            assert_eq!(
                decision.message,
                format!("Denied by the operator via chat: {}", unit.repeat(expected))
            );
        }
    }

    #[test]
    fn broker_should_word_the_notices_exactly_when_it_allows_or_denies() {
        let (mut broker, _) = make();
        bash(&mut broker);
        assert_eq!(
            broker.handle_reply(ME, CTX, "1").decision.unwrap().notice,
            "Allowed #a1."
        );
        bash(&mut broker);
        assert_eq!(
            broker.handle_reply(ME, CTX, "2").decision.unwrap().notice,
            "Allowed #a2, and the same call again in this session."
        );
        bash_in(&mut broker, "other", CTX, "tu");
        assert_eq!(
            broker.handle_reply(ME, CTX, "3").decision.unwrap().notice,
            "Denied #a3."
        );
    }

    #[test]
    fn broker_should_return_the_permission_result_with_keys_in_python_order() {
        let (mut broker, _) = make();
        bash(&mut broker);
        let allow = broker
            .handle_reply(ME, CTX, "1")
            .decision
            .unwrap()
            .permission_result();
        assert_eq!(
            allow.to_string(),
            r#"{"behavior":"allow","updatedInput":{"command":"touch probe.txt"}}"#
        );
        bash(&mut broker);
        let deny = broker
            .handle_reply(ME, CTX, "3")
            .decision
            .unwrap()
            .permission_result();
        assert_eq!(
            deny.to_string(),
            r#"{"behavior":"deny","message":"Denied by the operator via chat."}"#
        );
    }

    #[test]
    fn broker_should_answer_at_once_with_the_request_when_the_call_is_remembered() {
        let (mut broker, clock) = make_with(&[ME], 600.0, 20);
        bash_in(&mut broker, "make test", CTX, "tu1");
        broker.handle_reply(ME, CTX, "2");
        let again = bash_in(&mut broker, "make test", CTX, "tu2");
        assert!(again.request.is_none());
        let decision = again.decision.unwrap();
        assert_eq!(decision.request.request_id, "a2");
        assert_eq!(decision.request.deadline, clock.now() + 600.0);
        assert_eq!(
            (decision.message.as_str(), decision.notice.as_str()),
            ("", "")
        );
        assert_eq!(
            decision.permission_result(),
            json!({"behavior": "allow", "updatedInput": {"command": "make test"}})
        );
        assert!(broker.pending(None).is_empty());
    }

    #[test]
    fn broker_should_match_a_remembered_call_when_only_the_key_order_differs() {
        let (mut broker, _) = make();
        let first = json!({"a": 1, "b": {"x": 1, "y": [2, {"p": 1, "q": 2}]}});
        broker.submit(CTX, "Tool", &first, "tu1").unwrap();
        broker.handle_reply(ME, CTX, "2");
        let reordered = json!({"b": {"y": [2, {"q": 2, "p": 1}], "x": 1}, "a": 1});
        assert!(
            broker
                .submit(CTX, "Tool", &reordered, "tu2")
                .unwrap()
                .decision
                .is_some()
        );
    }

    #[test]
    fn broker_should_not_match_a_remembered_call_when_a_value_or_the_tool_differs() {
        // Python: json.dumps({"a": 1}) != json.dumps({"a": 1.0}); the tool name is part of the key.
        let (mut broker, _) = make();
        broker.submit(CTX, "Tool", &json!({"a": 1}), "tu1").unwrap();
        broker.handle_reply(ME, CTX, "2");
        assert!(
            broker
                .submit(CTX, "Tool", &json!({"a": 1.0}), "tu2")
                .unwrap()
                .decision
                .is_none()
        );
        broker.handle_reply(ME, CTX, "3 #a3");
        assert!(
            broker
                .submit(CTX, "Other", &json!({"a": 1}), "tu3")
                .unwrap()
                .decision
                .is_none()
        );
        assert!(
            broker
                .submit(CTX, "Tool", &json!({"a": 2}), "tu4")
                .unwrap()
                .decision
                .is_none()
        );
    }

    #[test]
    fn broker_should_treat_an_input_that_is_not_an_object_as_empty_when_it_submits() {
        for input in [json!("text"), json!([1, 2]), json!(null), json!(7)] {
            let (mut broker, _) = make();
            let sub = broker.submit(CTX, "Tool", &input, "tu1").unwrap();
            assert!(sub.request.unwrap().tool_input.is_empty(), "{input}");
            let result = broker
                .handle_reply(ME, CTX, "1")
                .decision
                .unwrap()
                .permission_result();
            assert_eq!(result, json!({"behavior": "allow", "updatedInput": {}}));
        }
    }

    #[test]
    fn broker_should_deny_everything_when_the_limit_is_zero() {
        let (mut broker, _) = make_with(&[ME], 900.0, 0);
        let sub = bash(&mut broker);
        assert_eq!(sub.decision.unwrap().reason, Reason::TooMany);
        assert!(sub.text.is_none());
    }

    #[test]
    fn broker_should_say_what_was_refused_when_it_denies_a_call_or_a_question() {
        let (mut broker, _) = make_with(&[ME], 900.0, 1);
        bash(&mut broker);
        let too_many = bash_in(&mut broker, "touch probe.txt", CTX, "tu2")
            .decision
            .unwrap();
        assert_eq!(
            too_many.message,
            "Too many approvals are waiting; the newest was denied."
        );
        assert_eq!(too_many.notice, too_many.message);
        assert_eq!(
            (too_many.request.request_id.as_str(), too_many.request.kind),
            ("a2", RequestKind::Tool)
        );
        let question_too_many = broker
            .submit_question(CTX, &question(), "tu3")
            .unwrap()
            .decision
            .unwrap();
        assert_eq!(question_too_many.reason, Reason::TooMany);
        assert_eq!(
            (
                question_too_many.request.tool_name.as_str(),
                question_too_many.request.kind
            ),
            ("AskUserQuestion", RequestKind::Question)
        );

        let (mut broker, _) = make();
        let malformed = broker
            .submit_question(CTX, &json!({}), "tu1")
            .unwrap()
            .decision
            .unwrap();
        assert_eq!(
            malformed.message,
            "The harness sent a question the bridge cannot show; denied."
        );
        assert_eq!(malformed.notice, malformed.message);
        assert_eq!(malformed.request.request_id, "a1");
        assert_eq!(
            malformed.permission_result(),
            json!({"behavior": "deny", "message": malformed.message})
        );
    }

    #[test]
    fn broker_should_fail_when_the_id_source_keeps_giving_taken_ids() {
        let calls = Arc::new(Mutex::new(0));
        let counted = Arc::clone(&calls);
        let ids = move || {
            *counted.lock().unwrap() += 1;
            "a1".to_owned()
        };
        let mut broker = Broker::new([ME], Box::new(|| 1000.0), Box::new(ids), 900.0, 20);
        bash(&mut broker);
        let err = broker
            .submit(CTX, "Bash", &json!({"command": "x"}), "tu2")
            .unwrap_err();
        assert_eq!(err.to_string(), "could not allocate a request id");
        // Python asks 1 + 100 times: the first id, then one more after each of the 100 taken ones.
        assert_eq!(*calls.lock().unwrap(), 1 + 101);
        assert_eq!(broker.pending(None).len(), 1);
    }

    #[test]
    fn broker_should_accept_an_id_found_on_the_last_try() {
        let mut taken = 0;
        let ids = move || {
            taken += 1;
            if taken <= 100 {
                "a1".to_owned()
            } else {
                "a2".to_owned()
            }
        };
        let mut broker = Broker::new([ME], Box::new(|| 1000.0), Box::new(ids), 900.0, 20);
        bash(&mut broker); // call 1: "a1" is free
        // The second request gets "a1" 100 times, which is every check Python makes, so "a2" comes from the 101st call.
        let second = bash_in(&mut broker, "touch probe.txt", CTX, "tu2");
        assert_eq!(second.request.unwrap().request_id, "a2");
    }

    #[test]
    fn broker_should_describe_a_question_exactly_when_it_shows_it() {
        let (mut broker, _) = make();
        let text = broker
            .submit_question(CTX, &question(), "tu1")
            .unwrap()
            .text
            .unwrap();
        assert_eq!(
            text,
            "Question #a1 (1 of 1): Which color?\n1. Red - warm\n2. Blue - cool\nReply with a number, or type your own answer."
        );
        let multi = broker
            .submit_question(CTX, &multi_question(), "tu2")
            .unwrap()
            .text
            .unwrap();
        assert_eq!(
            multi,
            "Question #a2 (1 of 1): Which?\n1. A\n2. B\n3. C\nReply with a number or several separated by commas, or type your own answer."
        );
    }

    #[test]
    fn broker_should_show_a_text_description_only_when_the_option_has_one() {
        let q = json!({"questions": [{"question": "Q?", "options": [
            {"label": "A", "description": ["no"]}, {"label": "B", "description": 5}, {"label": "C"}, {"label": "D", "description": "d token=abc123"}]}]});
        let (mut broker, _) = make();
        let text = broker
            .submit_question(CTX, &q, "tu1")
            .unwrap()
            .text
            .unwrap();
        assert!(text.contains("\n1. A\n2. B\n3. C\n4. D - d "), "{text}");
        assert!(!text.contains("abc123"));
    }

    #[test]
    fn broker_should_number_the_next_question_when_it_asks_the_second() {
        let two = json!({"questions": [question()["questions"][0].clone(), {"question": "Which size?", "options": [{"label": "S"}]}]});
        let (mut broker, _) = make();
        broker.submit_question(CTX, &two, "tu1").unwrap();
        assert_eq!(
            broker.handle_reply(ME, CTX, "1").notice,
            "Question #a1 (2 of 2): Which size?\n1. S\nReply with a number, or type your own answer."
        );
    }

    #[test]
    fn broker_should_say_what_is_invalid_when_the_number_is_wrong() {
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        for text in ["9", "0", "1,2", "1 2"] {
            assert_eq!(
                broker.handle_reply(ME, CTX, text).notice,
                "Choose a number from 1 to 2, or type your own answer.",
                "{text}"
            );
        }
        let (mut broker, _) = make();
        broker
            .submit_question(CTX, &multi_question(), "tu1")
            .unwrap();
        assert_eq!(
            broker.handle_reply(ME, CTX, "1,4").notice,
            "Choose a number from 1 to 3 (several allowed, separated by commas), or type your own answer."
        );
    }

    #[test]
    fn broker_should_read_unicode_digits_and_spaces_when_a_number_answers_a_question() {
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        let updated = broker
            .handle_reply(ME, CTX, "\u{662}")
            .decision
            .unwrap()
            .updated_input
            .unwrap();
        assert_eq!(updated["answers"], json!({"Which color?": "Blue"}));

        let (mut broker, _) = make();
        broker
            .submit_question(CTX, &multi_question(), "tu1")
            .unwrap();
        let updated = broker
            .handle_reply(ME, CTX, "1\u{1f}3 ,\u{ff12}, 1")
            .decision
            .unwrap()
            .updated_input
            .unwrap();
        assert_eq!(updated["answers"], json!({"Which?": "A, C, B, A"}));
    }

    #[test]
    fn broker_should_keep_waiting_when_the_number_is_huge() {
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        for text in ["9".repeat(30), "9".repeat(5000)] {
            assert_eq!(broker.handle_reply(ME, CTX, &text).kind, ReplyKind::Invalid);
        }
        assert_eq!(broker.pending(None).len(), 1);
    }

    #[test]
    fn broker_should_read_multi_select_as_python_does_when_the_flag_is_not_a_bool() {
        for (flag, several) in [
            (json!(1), true),
            (json!("yes"), true),
            (json!([0]), true),
            (json!(0), false),
            (json!(""), false),
            (json!(null), false),
        ] {
            let mut q = multi_question();
            q["questions"][0]["multiSelect"] = flag.clone();
            let (mut broker, _) = make();
            broker.submit_question(CTX, &q, "tu1").unwrap();
            let kind = broker.handle_reply(ME, CTX, "1,2").kind;
            assert_eq!(
                kind,
                if several {
                    ReplyKind::Decided
                } else {
                    ReplyKind::Invalid
                },
                "{flag}"
            );
        }
        let mut q = multi_question();
        q["questions"][0]
            .as_object_mut()
            .unwrap()
            .remove("multiSelect");
        let (mut broker, _) = make();
        broker.submit_question(CTX, &q, "tu1").unwrap();
        assert_eq!(broker.handle_reply(ME, CTX, "1,2").kind, ReplyKind::Invalid);
    }

    #[test]
    fn broker_should_keep_the_input_order_and_replace_answers_in_place_when_it_answers() {
        // Python: {**{"answers": {"z": 1}, "questions": [...]}, "answers": {...}} keeps "answers" first.
        let mut input = question();
        let questions = input["questions"].take();
        let input = json!({"answers": {"z": 1}, "questions": questions, "extra": true});
        let (mut broker, _) = make();
        broker.submit_question(CTX, &input, "tu1").unwrap();
        let updated = broker
            .handle_reply(ME, CTX, "1")
            .decision
            .unwrap()
            .permission_result();
        let keys: Vec<&str> = updated["updatedInput"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, ["answers", "questions", "extra"]);
        assert_eq!(
            updated["updatedInput"]["answers"],
            json!({"Which color?": "Red"})
        );
    }

    #[test]
    fn broker_should_keep_one_answer_when_two_questions_have_the_same_text() {
        // Python: d["q"] = "1"; d["r"] = "2"; d["q"] = "3" gives {"q": "3", "r": "2"}.
        let q = |text: &str| json!({"question": text, "options": [{"label": "A"}, {"label": "B"}]});
        let input = json!({"questions": [q("q"), q("r"), q("q")]});
        let (mut broker, _) = make();
        broker.submit_question(CTX, &input, "tu1").unwrap();
        broker.handle_reply(ME, CTX, "1");
        broker.handle_reply(ME, CTX, "2");
        let updated = broker
            .handle_reply(ME, CTX, "1")
            .decision
            .unwrap()
            .updated_input
            .unwrap();
        assert_eq!(
            serde_json::to_string(&updated["answers"]).unwrap(),
            r#"{"q":"A","r":"B"}"#
        );
    }

    #[test]
    fn broker_should_call_a_stranger_forbidden_when_a_question_waits_and_the_text_is_not_empty() {
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        assert_eq!(
            broker
                .handle_reply("mallory@chat.example", CTX, "lunch?")
                .kind,
            ReplyKind::Forbidden
        );
        assert_eq!(
            broker.handle_reply(ME, CTX, " \u{1f} ").kind,
            ReplyKind::NotAReply
        );
        assert_eq!(broker.pending(None).len(), 1);
    }

    #[test]
    fn broker_should_name_the_decision_when_a_question_is_answered() {
        let (mut broker, _) = make();
        broker.submit_question(CTX, &question(), "tu1").unwrap();
        let decision = broker.handle_reply(ME, CTX, "1").decision.unwrap();
        assert_eq!(
            (decision.reason, decision.behavior),
            (Reason::Answered, Behavior::Allow)
        );
        assert_eq!(decision.notice, "Answered #a1.");
    }

    #[test]
    fn broker_should_validate_a_question_when_some_part_is_not_the_right_type() {
        for bad in [
            json!({"questions": [{"question": 1, "options": [{"label": "A"}]}]}),
            json!({"questions": [{"question": "Q?", "options": "A"}]}),
            json!({"questions": [{"question": "Q?", "options": ["A"]}]}),
            json!({"questions": [{"question": "Q?", "options": [{"label": 1}]}]}),
            json!({"questions": ["Q?"]}),
            json!({"questions": [question()["questions"][0].clone(), {"question": "Q?"}]}),
            json!("questions"),
            json!(null),
        ] {
            let (mut broker, _) = make();
            let decision = broker
                .submit_question(CTX, &bad, "tu1")
                .unwrap()
                .decision
                .unwrap();
            assert_eq!(decision.reason, Reason::Malformed, "{bad}");
        }
        let fine = json!({"questions": [{"question": "", "options": [{"label": ""}]}]});
        let (mut broker, _) = make();
        assert!(
            broker
                .submit_question(CTX, &fine, "tu1")
                .unwrap()
                .decision
                .is_none()
        );
    }

    #[test]
    fn broker_should_show_its_pending_count_when_it_is_printed() {
        let (mut broker, _) = make();
        bash(&mut broker);
        assert_eq!(format!("{broker:?}"), "Broker { pending: 1, .. }");
    }

    #[test]
    fn as_str_should_give_the_python_names_when_a_kind_or_behavior_is_printed() {
        assert_eq!(
            (Behavior::Allow.as_str(), Behavior::Deny.as_str()),
            ("allow", "deny")
        );
        let kinds = [
            (ReplyKind::Decided, "decided"),
            (ReplyKind::Next, "next"),
            (ReplyKind::NotAReply, "not_a_reply"),
            (ReplyKind::Forbidden, "forbidden"),
            (ReplyKind::UnknownRequest, "unknown_request"),
            (ReplyKind::Ambiguous, "ambiguous"),
            (ReplyKind::Invalid, "invalid"),
        ];
        for (kind, name) in kinds {
            assert_eq!(kind.as_str(), name);
        }
        let reasons = [
            (Reason::Approved, "approved"),
            (Reason::ApprovedSession, "approved_session"),
            (Reason::Remembered, "remembered"),
            (Reason::Denied, "denied"),
            (Reason::Timeout, "timeout"),
            (Reason::Shutdown, "shutdown"),
            (Reason::TooMany, "too_many"),
            (Reason::Malformed, "malformed"),
            (Reason::Answered, "answered"),
        ];
        for (reason, name) in reasons {
            assert_eq!(reason.as_str(), name);
        }
    }

    #[test]
    fn broker_should_refuse_a_sender_when_only_a_newer_unicode_lowercases_it_to_the_approver() {
        // Python 3.13 leaves U+A7CB as it is, so it is not the approver "\u{264}@chat.example"; a newer Unicode lowercases it to U+0264.
        let (mut broker, _) = make_with(&["\u{264}@chat.example"], 900.0, 20);
        bash(&mut broker);
        assert_eq!(
            broker.handle_reply("\u{a7cb}@chat.example", CTX, "1").kind,
            ReplyKind::Forbidden
        );
        assert_eq!(
            broker.handle_reply("\u{264}@chat.example", CTX, "1").kind,
            ReplyKind::Decided
        );
    }
}
