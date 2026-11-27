#!/usr/bin/env python3
"""Mutation check: apply one small change to a module at a time and require the test suite to fail.
Usage: python3 scripts/mutation-check.py   (exit 1 when a mutation survives). Each entry is (module, label, old, new);
the module is "package/name", for example "xmpp_agent_client/router"."""
import os, shutil, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MUTATIONS = [
    # approvals: who may answer
    ("xmpp_agent_client/approvals", "approver check removed", "is_approver = _bare(sender) in self._approvers", "is_approver = True"),
    ("xmpp_agent_client/approvals", "resource not stripped", 'return jid.split("/", 1)[0].strip().lower()', "return jid.strip().lower()"),
    ("xmpp_agent_client/approvals", "jid case matters", 'return jid.split("/", 1)[0].strip().lower()', 'return jid.split("/", 1)[0].strip()'),
    # resolution
    ("xmpp_agent_client/approvals", "a request can decide twice", "        self._pending.pop(state.request.request_id, None)\n", ""),
    ("xmpp_agent_client/approvals", "late reply still decides", "    def handle_reply(self, sender, context, text):\n        self._expire()\n", "    def handle_reply(self, sender, context, text):\n"),
    ("xmpp_agent_client/approvals", "id works across contexts", "if state is None or state.request.context != context:", "if state is None:"),
    ("xmpp_agent_client/approvals", "several pending are not ambiguous", "if len(in_context) > 1:", "if False:"),
    ("xmpp_agent_client/approvals", "timeout never expires", "if now >= state.request.deadline:", "if False:"),
    ("xmpp_agent_client/approvals", "timeout allows", 'self._due.append(self._finish(state, "deny", "timeout"', 'self._due.append(self._finish(state, "allow", "timeout"'),
    ("xmpp_agent_client/approvals", "shutdown allows", 'out.append(self._finish(state, "deny", "shutdown"', 'out.append(self._finish(state, "allow", "shutdown"'),
    ("xmpp_agent_client/approvals", "close drops expired decisions", "        out, self._due = self._due, []\n        for state", "        out, self._due = [], []\n        for state"),
    # what counts as an answer
    ("xmpp_agent_client/approvals", "yes plus chatter counts as allow", 'if first in _ALLOW_WORDS and not rest:', 'if first in _ALLOW_WORDS:'),
    ("xmpp_agent_client/approvals", "chat from a stranger is flagged", "        if not self._looks_like_answer(state, body):\n            return Reply(\"not_a_reply\")\n        if not is_approver:\n            return Reply(\"forbidden\")\n",
     "        if not is_approver:\n            return Reply(\"forbidden\")\n        if not self._looks_like_answer(state, body):\n            return Reply(\"not_a_reply\")\n"),
    ("xmpp_agent_client/approvals", "deny note dropped", 'message = f"{DENY_MESSAGE[:-1]}: {note[:300]}" if note else DENY_MESSAGE', "message = DENY_MESSAGE"),
    # session memory and caps
    ("xmpp_agent_client/approvals", "memory ignores the context", "self._remembered.add((state.request.context, _call_key(", "self._remembered.add((\"\", _call_key("),
    ("xmpp_agent_client/approvals", "memory ignores the input", "return tool_name + json.dumps(tool_input, sort_keys=True, default=str)", "return tool_name"),
    ("xmpp_agent_client/approvals", "plan approval is remembered", 'if action == "session" and state.request.tool_name != "ExitPlanMode":', 'if action == "session":'),
    ("xmpp_agent_client/approvals", "forget does nothing", "        self._remembered = {item for item in self._remembered if item[0] != context}", "        pass"),
    ("xmpp_agent_client/approvals", "pending cap off by one", "if len(self._pending) >= self._max_pending:\n            return self._refuse(context, \"tool\"", "if len(self._pending) > self._max_pending:\n            return self._refuse(context, \"tool\""),
    ("xmpp_agent_client/approvals", "id collisions allowed", "            if rid not in self._pending and rid not in self._resolved:\n                break\n", "            break\n"),
    # questions
    ("xmpp_agent_client/approvals", "number 0 accepted", "if any(n < 1 or n > len(options) for n in picks)", "if any(n > len(options) for n in picks)"),
    ("xmpp_agent_client/approvals", "several numbers on a single select", " or (len(picks) > 1 and not q.get(\"multiSelect\")):", ":"),
    ("xmpp_agent_client/approvals", "answers keyed by header", 'state.answers[q["question"]] = answer', 'state.answers[q.get("header", "")] = answer'),
    ("xmpp_agent_client/approvals", "question text not redacted", "{redact(q['question'])}", "{q['question']}"),
    ("xmpp_agent_client/approvals", "answer dropped", 'updated = {**state.request.tool_input, "answers": dict(state.answers)}', "updated = dict(state.request.tool_input)"),
    ("xmpp_agent_client/approvals", "malformed question allowed", 'return self._refuse(context, "question", "AskUserQuestion", tool_input, tool_use_id, "malformed")', "pass"),
    # router
    ("xmpp_agent_client/router", "no path guard", "    if token and not _NAME.fullmatch(token):\n        return Prompt(text)  # a path or other text that merely starts with a slash\n", ""),
    ("xmpp_agent_client/router", "bang without a name is a command", "text[1].isalnum()", "True"),
    ("xmpp_agent_client/router", "harness command beats local", "    if lowered in LOCAL_COMMANDS:\n        return Local(lowered, args)\n    if token in terminal_commands:", "    if token in terminal_commands:"),
    ("xmpp_agent_client/router", "terminal commands pass", "    if token in terminal_commands:\n        return Rejected(\"interactive\", token)\n", ""),
    ("xmpp_agent_client/router", "unknown becomes a prompt", "    known = sorted(set(harness_commands) | LOCAL_COMMANDS)\n    return Rejected(\"unknown\", token, tuple(difflib.get_close_matches(token, known, n=3, cutoff=0.6)))", "    return Prompt(text)"),
    ("xmpp_agent_client/router", "local names are case sensitive", "lowered = token.lower()", "lowered = token"),
    ("xmpp_agent_client/router", "args keep padding", "args = body[len(token):].strip()", "args = body[len(token):]"),
    ("xmpp_agent_client/router", "double slash not an escape", "    if text.startswith(\"//\"):\n        return text[2:]\n", ""),
    # redact: token shapes and structure
    ("xmpp_agent_common/redact", "private key kept", "-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----", "-----NEVER-----"),
    ("xmpp_agent_common/redact", "URL credentials kept", "(?<=://)[^/\\s:@]+:[^/\\s@]+@", "NEVERMATCH"),
    ("xmpp_agent_common/redact", "authorization header kept", "(authorization\\s*:\\s*)", "(neverauthorization\\s*:\\s*)"),
    ("xmpp_agent_common/redact", "bearer kept", "\\bbearer\\s+[A-Za-z0-9._~+/=-]{10,}", "\\bneverbearer\\s+"),
    ("xmpp_agent_common/redact", "aws key kept", "\\bAKIA[0-9A-Z]{16}\\b", "\\bNEVERMATCH\\b"),
    ("xmpp_agent_common/redact", "github token kept", "\\bgh[pousr]_[A-Za-z0-9]{20,}\\b|", "\\bnevergh_[A-Za-z0-9]{20,}\\b|"),
    ("xmpp_agent_common/redact", "slack token kept", "\\bxox[abprs]-", "\\bneverxox-"),
    ("xmpp_agent_common/redact", "sk- key kept", "\\bsk-[A-Za-z0-9_-]{16,}", "\\bnevsk-[A-Za-z0-9_-]{16,}"),
    ("xmpp_agent_common/redact", "jwt kept", "\\beyJ[A-Za-z0-9_-]{8,}", "\\bnevereyJ[A-Za-z0-9_-]{8,}"),
    ("xmpp_agent_common/redact", "flag values kept", "(--?[A-Za-z0-9_-]*\" + _KEYWORDS", "(--?nevernever\" + _KEYWORDS"),
    ("xmpp_agent_common/redact", "json-quoted keys kept", "([\\\"']?\\s*[=:]\\s*)", "(\\s*[=:]\\s*)"),
    ("xmpp_agent_common/redact", "private key block cut at a newline", ", re.DOTALL)", ")"),
    ("xmpp_agent_common/redact", "cut before redact", "    return _cut(redact(out), max_chars)", "    return redact(_cut(out, max_chars))"),
    ("xmpp_agent_common/redact", "write shows content", "out = f\"Write {g('file_path')} ({len(g('content'))} chars)\"", "out = f\"Write {g('file_path')} {g('content')}\""),
    ("xmpp_agent_common/redact", "edit shows old text", "(replace {len(g('old_string'))} chars", "(replace {g('old_string')}"),
    ("xmpp_agent_common/redact", "cut ignores the limit", "    if len(text) <= max_chars:\n        return text\n", "    return text\n"),
    ("xmpp_agent_common/redact", "per-argument values not redacted", "_cut(redact(_value(v)), 80)", "_cut(_value(v), 80)"),
]

def run(module, label, old, new):
    path = os.path.join(ROOT, "src", module + ".py")
    src = open(path).read()
    if src.count(old) != 1:
        return f"BROKEN {label}: pattern found {src.count(old)} times"
    d = tempfile.mkdtemp()
    shutil.copytree(os.path.join(ROOT, "src"), d, dirs_exist_ok=True, ignore=shutil.ignore_patterns("__pycache__"))
    open(os.path.join(d, module + ".py"), "w").write(src.replace(old, new))
    r = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=ROOT,
                       env={"PYTHONPATH": d, "PATH": os.environ["PATH"]}, capture_output=True, text=True)
    shutil.rmtree(d)
    return ("CAUGHT " if r.returncode else "MISSED ") + label

results = [run(*m) for m in MUTATIONS]
print("\n".join(results))
bad = [r for r in results if not r.startswith("CAUGHT")]
print(f"\n{len(results) - len(bad)} of {len(results)} mutations caught")
sys.exit(1 if bad else 0)
