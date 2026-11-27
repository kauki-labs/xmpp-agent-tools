"""Approvals and dialogs for a harness. Pure logic: the clock and the id source are arguments, so tests need no sleeps.

The harness blocks on `--permission-prompt-tool` until a decision comes back (docs/experiments.md E3: it waited 10.5 minutes without
a timeout). This module decides what the answer is. Rules:
- Only an approver's real bare JID counts. A MUC nickname is not a JID, so the bridge must pass the occupant's real JID.
- Fail closed: a timeout, a shutdown, too many pending requests, a malformed question and a stranger's reply all deny or change nothing.
- A request resolves at most once. Replies are matched by context (room and thread), and by `#id` when several are pending.
- Ordinary chat is never mistaken for a decision: only "1", "2", "3", "4 note", "y", "yes", "allow", "ok", "n", "no", "deny" count.
"""
from __future__ import annotations

import json
import re
import secrets
from collections import deque
from dataclasses import dataclass

from xmpp_agent_common.redact import describe_tool, redact

DENY_MESSAGE = "Denied by the operator via chat."
_ALLOW_WORDS = {"1", "y", "yes", "allow", "ok"}
_DENY_WORDS = {"3", "n", "no", "deny", "4"}
_ID_LEAD = re.compile(r"^#([A-Za-z0-9]+)\s+(.*)$", re.DOTALL)
_ID_TAIL = re.compile(r"^(.*?)\s+#([A-Za-z0-9]+)$", re.DOTALL)
_NUMBERS = re.compile(r"\d+(?:\s*[,\s]\s*\d+)*")


@dataclass(frozen=True)
class Request:
    request_id: str
    context: str
    kind: str  # "tool" or "question"
    tool_name: str
    tool_input: dict
    tool_use_id: str
    deadline: float


@dataclass(frozen=True)
class Decision:
    request: Request
    behavior: str  # "allow" or "deny"
    reason: str  # approved, approved_session, remembered, denied, timeout, shutdown, too_many, malformed, answered
    message: str = ""
    updated_input: dict | None = None
    notice: str = ""

    def permission_result(self):
        """The JSON the permission-prompt tool returns to claude (docs/experiments.md E3 and E4)."""
        if self.behavior == "allow":
            return {"behavior": "allow", "updatedInput": self.updated_input if self.updated_input is not None else self.request.tool_input}
        return {"behavior": "deny", "message": self.message}


@dataclass(frozen=True)
class Submitted:
    request: Request | None  # None when the answer came at once (remembered, too many, malformed)
    text: str | None  # what to post to the room; None when nothing is to be asked
    decision: Decision | None  # set when no human is needed


@dataclass(frozen=True)
class Reply:
    kind: str  # decided, next, not_a_reply, forbidden, unknown_request, ambiguous, invalid
    notice: str = ""
    decision: Decision | None = None


class _State:
    def __init__(self, request, questions=None):
        self.request = request
        self.questions = questions or []
        self.index = 0
        self.answers = {}


def _call_key(tool_name, tool_input):
    return tool_name + json.dumps(tool_input, sort_keys=True, default=str)


def _bare(jid):
    return jid.split("/", 1)[0].strip().lower()


def _parse_choice(text):
    """("allow" | "session" | "deny", note) or None when the text is not a decision."""
    parts = text.strip().split(None, 1)
    if not parts:
        return None
    first, rest = parts[0].lower().rstrip(".!"), (parts[1].strip() if len(parts) > 1 else "")
    if first in _ALLOW_WORDS and not rest:
        return ("allow", "")
    if first == "2" and not rest:
        return ("session", "")
    if first in _DENY_WORDS:
        return ("deny", rest)
    return None


def _split_id(text):
    text = text.strip()
    if (m := _ID_LEAD.match(text)):
        return m.group(1), m.group(2).strip()
    if (m := _ID_TAIL.match(text)):
        return m.group(2), m.group(1).strip()
    if re.fullmatch(r"#[A-Za-z0-9]+", text):
        return text[1:], ""
    return None, text


class Broker:
    def __init__(self, approvers, clock, ids=None, timeout=900.0, max_pending=20):
        self._approvers = {_bare(a) for a in approvers}
        self._clock = clock
        self._ids = ids or (lambda: secrets.token_hex(2))
        self._timeout = timeout
        self._max_pending = max_pending
        self._pending: dict[str, _State] = {}
        self._resolved = deque(maxlen=256)
        self._remembered: set = set()
        self._due: list[Decision] = []

    # -- public --------------------------------------------------------------------------------------------------------
    def pending(self, context=None):
        return [s.request for s in self._pending.values() if context is None or s.request.context == context]

    def forget(self, context):
        self._remembered = {item for item in self._remembered if item[0] != context}

    def submit(self, context, tool_name, tool_input, tool_use_id):
        tool_input = tool_input if isinstance(tool_input, dict) else {}
        if tool_name != "ExitPlanMode" and (context, _call_key(tool_name, tool_input)) in self._remembered:
            request = self._request(context, "tool", tool_name, tool_input, tool_use_id)
            return Submitted(None, None, Decision(request, "allow", "remembered"))
        if len(self._pending) >= self._max_pending:
            return self._refuse(context, "tool", tool_name, tool_input, tool_use_id, "too_many")
        request = self._request(context, "tool", tool_name, tool_input, tool_use_id)
        self._pending[request.request_id] = _State(request)
        minutes = round(self._timeout / 60)
        text = (f"Approval #{request.request_id}: {describe_tool(tool_name, tool_input)}\n"
                f"Reply 1 to allow once, 2 to allow this exact call again in this session, 3 to deny, or 4 and a note to deny with a reason. "
                f"It is denied after {minutes} min without an answer.")
        return Submitted(request, text, None)

    def submit_question(self, context, tool_input, tool_use_id):
        tool_input = tool_input if isinstance(tool_input, dict) else {}
        questions = tool_input.get("questions")
        valid = (isinstance(questions, list) and questions and all(
            isinstance(q, dict) and isinstance(q.get("question"), str) and isinstance(q.get("options"), list) and q["options"]
            and all(isinstance(o, dict) and isinstance(o.get("label"), str) for o in q["options"]) for q in questions))
        if not valid:
            return self._refuse(context, "question", "AskUserQuestion", tool_input, tool_use_id, "malformed")
        if len(self._pending) >= self._max_pending:
            return self._refuse(context, "question", "AskUserQuestion", tool_input, tool_use_id, "too_many")
        request = self._request(context, "question", "AskUserQuestion", tool_input, tool_use_id)
        state = _State(request, questions)
        self._pending[request.request_id] = state
        return Submitted(request, self._render_question(state), None)

    def handle_reply(self, sender, context, text):
        self._expire()
        rid, body = _split_id(text)
        is_approver = _bare(sender) in self._approvers
        in_context = [s for s in self._pending.values() if s.request.context == context]
        if rid is not None:
            state = self._pending.get(rid)
            if state is None or state.request.context != context:
                if _parse_choice(body) is None:
                    return Reply("not_a_reply")  # "#123" in ordinary chat
                if not is_approver:
                    return Reply("forbidden")
                gone = "is no longer pending (already decided or timed out)" if rid in self._resolved else "is not pending here"
                return Reply("unknown_request", f"Request #{rid} {gone}.")
        else:
            if not in_context:
                return Reply("not_a_reply")
            if len(in_context) > 1:
                if _parse_choice(body) is None and not _NUMBERS.fullmatch(body.strip()):
                    return Reply("not_a_reply")
                if not is_approver:
                    return Reply("forbidden")
                ids = ", ".join(f"#{s.request.request_id}" for s in in_context)
                return Reply("ambiguous", f"Several requests are waiting ({ids}). Add the id to your answer, for example `1 #{in_context[0].request.request_id}`.")
            state = in_context[0]
        if not self._looks_like_answer(state, body):
            return Reply("not_a_reply")
        if not is_approver:
            return Reply("forbidden")
        return self._answer(state, body)

    def tick(self):
        self._expire()
        due, self._due = self._due, []
        return due

    def close(self):
        out, self._due = self._due, []
        for state in list(self._pending.values()):
            out.append(self._finish(state, "deny", "shutdown", message="The bridge shut down before the operator answered.",
                                    notice=f"Approval #{state.request.request_id} was denied: the bridge is shutting down."))
        return out

    # -- internals -----------------------------------------------------------------------------------------------------
    def _request(self, context, kind, tool_name, tool_input, tool_use_id):
        rid = self._ids()
        for _ in range(100):
            if rid not in self._pending and rid not in self._resolved:
                break
            rid = self._ids()
        else:
            raise RuntimeError("could not allocate a request id")
        return Request(rid, context, kind, tool_name, tool_input, tool_use_id, self._clock() + self._timeout)

    def _refuse(self, context, kind, tool_name, tool_input, tool_use_id, reason):
        request = self._request(context, kind, tool_name, tool_input, tool_use_id)
        notice = {"too_many": "Too many approvals are waiting; the newest was denied.",
                  "malformed": "The harness sent a question the bridge cannot show; denied."}[reason]
        return Submitted(None, None, Decision(request, "deny", reason, message=notice, notice=notice))

    def _finish(self, state, behavior, reason, message="", updated_input=None, notice=""):
        self._pending.pop(state.request.request_id, None)
        self._resolved.append(state.request.request_id)
        return Decision(state.request, behavior, reason, message=message, updated_input=updated_input, notice=notice)

    def _expire(self):
        now = self._clock()
        for state in list(self._pending.values()):
            if now >= state.request.deadline:
                rid = state.request.request_id
                self._due.append(self._finish(state, "deny", "timeout", message="Timed out waiting for the operator via chat.",
                                              notice=f"Approval #{rid} timed out after {round(self._timeout / 60)} min and was denied."))

    def _looks_like_answer(self, state, body):
        if state.request.kind == "tool":
            return _parse_choice(body) is not None
        return bool(body.strip())

    def _answer(self, state, body):
        rid = state.request.request_id
        if state.request.kind == "question":
            return self._answer_question(state, body)
        action, note = _parse_choice(body)
        if action == "deny":
            message = f"{DENY_MESSAGE[:-1]}: {note[:300]}" if note else DENY_MESSAGE
            return Reply("decided", decision=self._finish(state, "deny", "denied", message=message, notice=f"Denied #{rid}."))
        if action == "session" and state.request.tool_name != "ExitPlanMode":
            self._remembered.add((state.request.context, _call_key(state.request.tool_name, state.request.tool_input)))
            return Reply("decided", decision=self._finish(state, "allow", "approved_session",
                                                          notice=f"Allowed #{rid}, and the same call again in this session."))
        return Reply("decided", decision=self._finish(state, "allow", "approved", notice=f"Allowed #{rid}."))

    def _answer_question(self, state, body):
        q = state.questions[state.index]
        options = q["options"]
        text = body.strip()
        if _NUMBERS.fullmatch(text):
            picks = [int(n) for n in re.split(r"[,\s]+", text)]
            if any(n < 1 or n > len(options) for n in picks) or (len(picks) > 1 and not q.get("multiSelect")):
                several = " (several allowed, separated by commas)" if q.get("multiSelect") else ""
                return Reply("invalid", f"Choose a number from 1 to {len(options)}{several}, or type your own answer.")
            answer = ", ".join(options[n - 1]["label"] for n in picks)
        else:
            answer = text
        state.answers[q["question"]] = answer
        state.index += 1
        if state.index < len(state.questions):
            return Reply("next", self._render_question(state))
        updated = {**state.request.tool_input, "answers": dict(state.answers)}
        return Reply("decided", decision=self._finish(state, "allow", "answered", updated_input=updated,
                                                      notice=f"Answered #{state.request.request_id}."))

    def _render_question(self, state):
        q = state.questions[state.index]
        rid = state.request.request_id
        lines = [f"Question #{rid} ({state.index + 1} of {len(state.questions)}): {redact(q['question'])}"]
        for n, option in enumerate(q["options"], 1):
            desc = option.get("description")
            lines.append(f"{n}. {redact(option['label'])}" + (f" - {redact(desc)}" if desc else ""))
        several = " or several separated by commas" if q.get("multiSelect") else ""
        lines.append(f"Reply with a number{several}, or type your own answer.")
        return "\n".join(lines)
