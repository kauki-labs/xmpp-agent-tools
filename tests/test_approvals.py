import itertools
import unittest

from xmpp_agent_client.approvals import Broker

ME = "me@chat.example"
CTX = "engineering#t1"


class Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


def make(approvers=(ME,), timeout=900.0, max_pending=20):
    clock = Clock()
    counter = itertools.count(1)
    broker = Broker(set(approvers), clock, lambda: f"a{next(counter)}", timeout=timeout, max_pending=max_pending)
    return broker, clock


def bash(broker, command="touch probe.txt", ctx=CTX, tool_use_id="tu1"):
    return broker.submit(ctx, "Bash", {"command": command}, tool_use_id)


class Submit(unittest.TestCase):
    def test_message_has_the_id_the_redacted_call_and_the_choices(self):
        broker, _ = make()
        sub = bash(broker, "DB_PASSWORD=hunter2 psql -h db")
        self.assertIsNone(sub.decision)
        self.assertEqual(sub.request.request_id, "a1")
        self.assertIn("#a1", sub.text)
        self.assertIn("psql -h db", sub.text)
        self.assertNotIn("hunter2", sub.text)
        for choice in ("1", "2", "3"):
            self.assertIn(choice, sub.text)

    def test_request_carries_its_deadline(self):
        broker, clock = make(timeout=600)
        self.assertEqual(bash(broker).request.deadline, clock.now + 600)


class Replies(unittest.TestCase):
    def test_one_allows_once_and_maps_to_the_permission_tool_result(self):
        broker, _ = make()
        bash(broker)
        rep = broker.handle_reply(ME, CTX, "1")
        self.assertEqual(rep.kind, "decided")
        self.assertEqual(rep.decision.behavior, "allow")
        self.assertEqual(rep.decision.permission_result(), {"behavior": "allow", "updatedInput": {"command": "touch probe.txt"}})
        self.assertEqual(broker.pending(), [])

    def test_words_work_like_numbers(self):
        for word, behavior in (("y", "allow"), ("yes", "allow"), ("allow", "allow"), ("n", "deny"), ("no", "deny"), ("deny", "deny"), ("3", "deny")):
            broker, _ = make()
            bash(broker)
            self.assertEqual(broker.handle_reply(ME, CTX, word).decision.behavior, behavior, word)

    def test_deny_gives_the_model_a_message(self):
        broker, _ = make()
        bash(broker)
        res = broker.handle_reply(ME, CTX, "3").decision.permission_result()
        self.assertEqual(res, {"behavior": "deny", "message": "Denied by the operator via chat."})

    def test_four_denies_with_a_note(self):
        broker, _ = make()
        bash(broker)
        res = broker.handle_reply(ME, CTX, "4 too risky, use a temp dir").decision.permission_result()
        self.assertEqual(res["behavior"], "deny")
        self.assertIn("too risky, use a temp dir", res["message"])

    def test_a_stranger_cannot_decide(self):
        broker, _ = make()
        bash(broker)
        rep = broker.handle_reply("mallory@chat.example", CTX, "1")
        self.assertEqual(rep.kind, "forbidden")
        self.assertIsNone(rep.decision)
        self.assertEqual(len(broker.pending()), 1)

    def test_no_approvers_means_nobody_decides(self):
        broker, _ = make(approvers=())
        bash(broker)
        self.assertEqual(broker.handle_reply(ME, CTX, "1").kind, "forbidden")

    def test_the_resource_and_the_case_of_the_jid_do_not_matter(self):
        broker, _ = make()
        bash(broker)
        self.assertEqual(broker.handle_reply("ME@Chat.Example/phone", CTX, "1").kind, "decided")

    def test_a_polite_yes_is_not_an_approval(self):
        # only a bare allow word counts; "yes but check first" must not approve
        for text in ("yes please", "ok but check the path first", "1 please", "allow it if the tests pass"):
            broker, _ = make()
            bash(broker)
            self.assertEqual(broker.handle_reply(ME, CTX, text).kind, "not_a_reply", text)
            self.assertEqual(len(broker.pending()), 1)

    def test_a_nickname_is_not_a_jid(self):
        broker, _ = make()
        bash(broker)
        self.assertEqual(broker.handle_reply("engineering@rooms.chat.example/me", CTX, "1").kind, "forbidden")

    def test_ordinary_chat_from_the_approver_is_not_a_reply(self):
        broker, _ = make()
        bash(broker)
        self.assertEqual(broker.handle_reply(ME, CTX, "why does it need that?").kind, "not_a_reply")
        self.assertEqual(len(broker.pending()), 1)

    def test_ordinary_chat_from_a_stranger_is_not_flagged(self):
        broker, _ = make()
        bash(broker)
        self.assertEqual(broker.handle_reply("mallory@chat.example", CTX, "lunch?").kind, "not_a_reply")

    def test_nothing_pending_means_not_a_reply(self):
        broker, _ = make()
        self.assertEqual(broker.handle_reply(ME, CTX, "1").kind, "not_a_reply")

    def test_contexts_are_isolated(self):
        broker, _ = make()
        bash(broker, ctx="engineering#t1")
        self.assertEqual(broker.handle_reply(ME, "engineering#t2", "1").kind, "not_a_reply")
        self.assertEqual(len(broker.pending()), 1)


class Ids(unittest.TestCase):
    def test_two_pending_need_an_id(self):
        broker, _ = make()
        bash(broker, tool_use_id="tu1")
        bash(broker, tool_use_id="tu2")
        rep = broker.handle_reply(ME, CTX, "1")
        self.assertEqual(rep.kind, "ambiguous")
        self.assertIn("#a1", rep.notice)
        self.assertIn("#a2", rep.notice)
        self.assertEqual(len(broker.pending()), 2)

    def test_the_id_may_come_before_or_after_the_answer(self):
        broker, _ = make()
        bash(broker, tool_use_id="tu1")
        bash(broker, tool_use_id="tu2")
        self.assertEqual(broker.handle_reply(ME, CTX, "1 #a2").decision.request.request_id, "a2")
        self.assertEqual(broker.handle_reply(ME, CTX, "#a1 3").decision.request.request_id, "a1")

    def test_an_unknown_id_is_reported_and_changes_nothing(self):
        broker, _ = make()
        bash(broker)
        rep = broker.handle_reply(ME, CTX, "1 #zz")
        self.assertEqual(rep.kind, "unknown_request")
        self.assertEqual(len(broker.pending()), 1)

    def test_a_request_decides_at_most_once(self):
        broker, _ = make()
        bash(broker)
        self.assertEqual(broker.handle_reply(ME, CTX, "1 #a1").kind, "decided")
        again = broker.handle_reply(ME, CTX, "3 #a1")
        self.assertEqual(again.kind, "unknown_request")
        self.assertIn("no longer pending", again.notice)
        self.assertIsNone(again.decision)

    def test_an_id_belongs_to_its_context(self):
        broker, _ = make()
        bash(broker, ctx="engineering#t1")
        self.assertEqual(broker.handle_reply(ME, "engineering#t2", "1 #a1").kind, "unknown_request")
        self.assertEqual(len(broker.pending()), 1)


class Timeouts(unittest.TestCase):
    def test_nothing_expires_early(self):
        broker, clock = make(timeout=900)
        bash(broker)
        clock.now += 899
        self.assertEqual(broker.tick(), [])

    def test_a_timeout_denies(self):
        broker, clock = make(timeout=900)
        bash(broker)
        clock.now += 901
        decisions = broker.tick()
        self.assertEqual(len(decisions), 1)
        self.assertEqual(decisions[0].behavior, "deny")
        self.assertEqual(decisions[0].reason, "timeout")
        self.assertIn("#a1", decisions[0].notice)
        self.assertEqual(decisions[0].permission_result()["behavior"], "deny")
        self.assertEqual(broker.pending(), [])
        self.assertEqual(broker.tick(), [])  # once only

    def test_a_late_reply_is_refused(self):
        broker, clock = make(timeout=900)
        bash(broker)
        clock.now += 901
        rep = broker.handle_reply(ME, CTX, "1")  # lazy expiry: the reply path expires first
        self.assertNotEqual(rep.kind, "decided")
        self.assertEqual(broker.pending(), [])

    def test_close_denies_everything_pending(self):
        broker, _ = make()
        bash(broker, tool_use_id="tu1")
        bash(broker, ctx="other#t", tool_use_id="tu2")
        decisions = broker.close()
        self.assertEqual({d.reason for d in decisions}, {"shutdown"})
        self.assertEqual({d.behavior for d in decisions}, {"deny"})
        self.assertEqual(len(decisions), 2)
        self.assertEqual(broker.pending(), [])

    def test_close_still_reports_what_had_already_expired(self):
        broker, clock = make(timeout=900)
        bash(broker, tool_use_id="tu1")
        clock.now += 901
        broker.handle_reply(ME, CTX, "hello")  # expires lazily, nobody ticked yet
        bash(broker, tool_use_id="tu2")
        reasons = sorted(d.reason for d in broker.close())
        self.assertEqual(reasons, ["shutdown", "timeout"])


class AllowForSession(unittest.TestCase):
    def test_two_remembers_the_exact_call(self):
        broker, _ = make()
        bash(broker, "make test")
        self.assertEqual(broker.handle_reply(ME, CTX, "2").decision.reason, "approved_session")
        again = bash(broker, "make test", tool_use_id="tu2")
        self.assertEqual(again.decision.behavior, "allow")
        self.assertEqual(again.decision.reason, "remembered")
        self.assertIsNone(again.text)

    def test_a_different_call_or_context_still_asks(self):
        broker, _ = make()
        bash(broker, "make test")
        broker.handle_reply(ME, CTX, "2")
        self.assertIsNone(bash(broker, "make deploy", tool_use_id="tu2").decision)
        self.assertIsNone(bash(broker, "make test", ctx="other#t", tool_use_id="tu3").decision)

    def test_forget_clears_the_memory(self):
        broker, _ = make()
        bash(broker, "make test")
        broker.handle_reply(ME, CTX, "2")
        broker.forget(CTX)
        self.assertIsNone(bash(broker, "make test", tool_use_id="tu2").decision)

    def test_plan_approval_is_never_remembered(self):
        broker, _ = make()
        broker.submit(CTX, "ExitPlanMode", {"plan": "do things"}, "tu1")
        self.assertEqual(broker.handle_reply(ME, CTX, "2").decision.reason, "approved")
        self.assertIsNone(broker.submit(CTX, "ExitPlanMode", {"plan": "do things"}, "tu2").decision)


class IdAllocation(unittest.TestCase):
    def test_a_colliding_id_is_replaced(self):
        clock = Clock()
        source = iter(["a1", "a1", "a1", "a2"])
        broker = Broker({ME}, clock, lambda: next(source))
        first = bash(broker, tool_use_id="tu1")
        second = bash(broker, tool_use_id="tu2")
        self.assertEqual(first.request.request_id, "a1")
        self.assertEqual(second.request.request_id, "a2")

    def test_a_resolved_id_is_not_reused(self):
        clock = Clock()
        source = iter(["a1", "a1", "a2"])
        broker = Broker({ME}, clock, lambda: next(source))
        bash(broker)
        broker.handle_reply(ME, CTX, "1")
        self.assertEqual(bash(broker, tool_use_id="tu2").request.request_id, "a2")


class Limits(unittest.TestCase):
    def test_too_many_pending_is_denied_at_once(self):
        broker, _ = make(max_pending=2)
        bash(broker, tool_use_id="tu1")
        bash(broker, tool_use_id="tu2")
        third = bash(broker, tool_use_id="tu3")
        self.assertEqual(third.decision.behavior, "deny")
        self.assertEqual(third.decision.reason, "too_many")
        self.assertEqual(len(broker.pending()), 2)


QUESTION = {"questions": [{"question": "Which color?", "header": "Color", "multiSelect": False,
                           "options": [{"label": "Red", "description": "warm"}, {"label": "Blue", "description": "cool"}]}]}


class Questions(unittest.TestCase):
    def test_malformed_questions_are_denied_not_shown(self):
        for bad in ({}, {"questions": []}, {"questions": "x"}, {"questions": [{"question": "Q?"}]},
                    {"questions": [{"question": "Q?", "options": []}]}, {"questions": [{"question": "Q?", "options": [{"description": "no label"}]}]}):
            broker, _ = make()
            sub = broker.submit_question(CTX, bad, "tu1")
            self.assertEqual(sub.decision.behavior, "deny", bad)
            self.assertEqual(sub.decision.reason, "malformed")
            self.assertIsNone(sub.text)
            self.assertEqual(broker.pending(), [])

    def test_several_numbers_on_a_single_select_are_invalid(self):
        broker, _ = make()
        broker.submit_question(CTX, QUESTION, "tu1")
        rep = broker.handle_reply(ME, CTX, "1,2")
        self.assertEqual(rep.kind, "invalid")
        self.assertEqual(len(broker.pending()), 1)

    def test_options_are_numbered_in_the_message(self):
        broker, _ = make()
        sub = broker.submit_question(CTX, QUESTION, "tu1")
        self.assertIn("Which color?", sub.text)
        self.assertIn("1", sub.text)
        self.assertIn("Red", sub.text)
        self.assertIn("2", sub.text)
        self.assertIn("Blue", sub.text)

    def test_a_number_answers_with_the_label(self):
        broker, _ = make()
        broker.submit_question(CTX, QUESTION, "tu1")
        rep = broker.handle_reply(ME, CTX, "2")
        self.assertEqual(rep.kind, "decided")
        # experiments E4: the answer goes back as updatedInput.answers, keyed by the question text
        self.assertEqual(rep.decision.permission_result(), {"behavior": "allow", "updatedInput": {**QUESTION, "answers": {"Which color?": "Blue"}}})

    def test_free_text_is_an_answer_too(self):
        broker, _ = make()
        broker.submit_question(CTX, QUESTION, "tu1")
        rep = broker.handle_reply(ME, CTX, "green, like the logo")
        self.assertEqual(rep.decision.updated_input["answers"], {"Which color?": "green, like the logo"})

    def test_an_out_of_range_number_is_invalid_and_keeps_waiting(self):
        broker, _ = make()
        broker.submit_question(CTX, QUESTION, "tu1")
        for bad in ("9", "0"):
            rep = broker.handle_reply(ME, CTX, bad)
            self.assertEqual(rep.kind, "invalid")
            self.assertEqual(len(broker.pending()), 1)

    def test_a_stranger_cannot_answer(self):
        broker, _ = make()
        broker.submit_question(CTX, QUESTION, "tu1")
        self.assertEqual(broker.handle_reply("mallory@chat.example", CTX, "1").kind, "forbidden")

    def test_several_questions_are_asked_one_after_the_other(self):
        two = {"questions": [QUESTION["questions"][0],
                             {"question": "Which size?", "header": "Size", "multiSelect": False, "options": [{"label": "S", "description": ""}, {"label": "L", "description": ""}]}]}
        broker, _ = make()
        broker.submit_question(CTX, two, "tu1")
        first = broker.handle_reply(ME, CTX, "1")
        self.assertEqual(first.kind, "next")
        self.assertIn("Which size?", first.notice)
        second = broker.handle_reply(ME, CTX, "2")
        self.assertEqual(second.kind, "decided")
        self.assertEqual(second.decision.updated_input["answers"], {"Which color?": "Red", "Which size?": "L"})

    def test_multi_select_joins_the_labels(self):
        # The joined format is a guess: experiments E4 covered single select only. Verify against claude before relying on it.
        multi = {"questions": [{"question": "Which?", "header": "H", "multiSelect": True,
                                "options": [{"label": "A", "description": ""}, {"label": "B", "description": ""}, {"label": "C", "description": ""}]}]}
        broker, _ = make()
        broker.submit_question(CTX, multi, "tu1")
        self.assertEqual(broker.handle_reply(ME, CTX, "1,3").decision.updated_input["answers"], {"Which?": "A, C"})

    def test_a_question_that_times_out_is_denied(self):
        broker, clock = make(timeout=900)
        broker.submit_question(CTX, QUESTION, "tu1")
        clock.now += 901
        self.assertEqual(broker.tick()[0].behavior, "deny")

    def test_questions_are_redacted(self):
        q = {"questions": [{"question": "Use token=abc123 for the call?", "header": "H", "multiSelect": False,
                            "options": [{"label": "Yes", "description": ""}, {"label": "No", "description": ""}]}]}
        broker, _ = make()
        self.assertNotIn("abc123", broker.submit_question(CTX, q, "tu1").text)


if __name__ == "__main__":
    unittest.main()
