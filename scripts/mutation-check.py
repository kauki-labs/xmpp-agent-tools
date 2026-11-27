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
    # ci: merge gate
    ("ci/merge_gate", "failure counts as green", 'GREEN = frozenset({"success", "skipped"})', 'GREEN = frozenset({"success", "skipped", "failure"})'),
    ("ci/merge_gate", "skipped counts as red", 'GREEN = frozenset({"success", "skipped"})', 'GREEN = frozenset({"success"})'),
    ("ci/merge_gate", "cancelled counts as green", 'GREEN = frozenset({"success", "skipped"})', 'GREEN = frozenset({"success", "skipped", "cancelled"})'),
    ("ci/merge_gate", "no jobs passes", "if not isinstance(needs, dict) or not needs:", "if not isinstance(needs, dict):"),
    ("ci/merge_gate", "the last job decides", "        ok = ok and green\n", "        ok = green\n"),
    ("ci/merge_gate", "exit code always zero", "    return 0 if ok else 1\n", "    return 0\n"),
    ("ci/merge_gate", "bad json passes", "    except (TypeError, ValueError):\n        print(\"merge gate: NEEDS_JSON is missing or not JSON, failing closed\", file=sys.stderr)\n        return 1\n",
     "    except (TypeError, ValueError):\n        return 0\n"),
    ("ci/merge_gate", "a malformed entry is read as green", "    return entry.get(\"result\") if isinstance(entry, dict) else None", "    return entry.get(\"result\") if isinstance(entry, dict) else \"success\""),
    # ci: changed-path classifier
    ("ci/changes", "markdown is no longer docs", '_DOC_SUFFIXES = (".md",)', '_DOC_SUFFIXES = (".mdx",)'),
    ("ci/changes", "license is code", 'return path == "LICENSE" or path.endswith(_DOC_SUFFIXES)', "return path.endswith(_DOC_SUFFIXES)"),
    ("ci/changes", "an empty diff skips everything", "        return {\"code\": True, \"workflows\": True, \"rust\": True}", "        return {\"code\": False, \"workflows\": False, \"rust\": False}"),
    ("ci/changes", "workflow prefix typo", '    workflows = any(p.startswith(".github/") for p in paths)', '    workflows = any(p.startswith(".githubx/") for p in paths)'),
    ("ci/changes", "workflow files count as code", 'code = any(not p.startswith(".github/") and not _is_doc(p) for p in paths)', "code = any(not _is_doc(p) for p in paths)"),
    ("ci/changes", "docs always count as docs", 'code = any(not p.startswith(".github/") and not _is_doc(p) for p in paths)', 'code = any(not p.startswith(".github/") and not _is_doc(p) and not p.startswith("docs/") for p in paths)'),
    ("ci/changes", "blank lines count as paths", "    paths = [p.strip() for p in paths if p and p.strip()]", "    paths = [p for p in paths if p]"),
    ("ci/changes", "rust sources are not rust", 'return path.endswith(".rs") or', 'return path.endswith(".rss") or'),
    ("ci/changes", "cargo manifests are not rust", '"Cargo.toml", "Cargo.lock", ', ""),
    ("ci/changes", "the cargo directory is not rust", 'or path.startswith(".cargo/")', "or False"),
    ("ci/changes", "outputs are inverted", "'true' if value else 'false'", "'false' if value else 'true'"),
    # ci: scrub check
    ("ci/scrub", "domain rule off", '("internal-domain", re.compile(r"kauki\\.xyz", re.IGNORECASE)),', '("internal-domain", re.compile(r"neverneverxyz", re.IGNORECASE)),'),
    ("ci/scrub", "10.x range off", 'rf"10\\.{_OCTET}', 'rf"11\\.{_OCTET}'),
    ("ci/scrub", "192.168 range off", 'rf"|192\\.168\\.', 'rf"|192\\.169\\.'),
    ("ci/scrub", "172.16 range too narrow", "172\\.(?:1[6-9]|2\\d|3[01])", "172\\.(?:1[6-9])"),
    ("ci/scrub", "mesh range off", "100\\.(?:6[4-9]|[7-9]\\d|1[01]\\d|12[0-7])", "100\\.(?:6[4-9])"),
    ("ci/scrub", "link-local off", 'rf"|169\\.254\\.', 'rf"|169\\.255\\.'),
    ("ci/scrub", "number-boundary guard dropped", 'r"(?<![\\d.])(?:"', 'r"(?:"'),
    ("ci/scrub", "trailing boundary dropped", 'r")(?![\\d.]*\\d)")),', 'r")")),'),
    ("ci/scrub", "home path rule off", '("home-path", re.compile(r"/(?:Users|home)/', '("home-path", re.compile(r"/(?:Nobody|home1)/'),
    ("ci/scrub", "runner exemption dropped", "(?!runner\\b|user\\b|example\\b)", "(?!user\\b|example\\b)"),
    ("ci/scrub", "personal rule case-sensitive", "\\.net\"]), re.IGNORECASE)),", "\\.net\"]))),"),
    ("ci/scrub", "line numbers start at 0", "enumerate(text.splitlines(), 1)", "enumerate(text.splitlines(), 0)"),
    ("ci/scrub", "codeowners may name anything", '_RELAXED = {"CODEOWNERS": frozenset({"personal"})}', '_RELAXED = {"CODEOWNERS": frozenset({"personal", "private-address", "internal-domain", "home-path"})}'),
    ("ci/scrub", "the codeowners exemption is gone", '_RELAXED = {"CODEOWNERS": frozenset({"personal"})}', '_RELAXED = {}'),
    ("ci/scrub", "the exemption applies to every file", 'relaxed = _RELAXED.get(os.path.basename(rel), frozenset())', 'relaxed = frozenset({"personal"})'),
    ("ci/scrub", "license is scanned", '_SKIP_NAMES = frozenset({"LICENSE"})', '_SKIP_NAMES = frozenset()'),
    ("ci/scrub", "binary files are scanned", '        if b"\\0" in data:\n            continue  # binary\n', ""),
    ("ci/scrub", "exit code always zero", "    if problems:\n        print(f\"\\nscrub:", "    if False:\n        print(f\"\\nscrub:"),
]

def run(module, label, old, new):
    rel = module + ".py" if module.startswith("ci/") else os.path.join("src", module + ".py")
    src = open(os.path.join(ROOT, rel)).read()
    if src.count(old) != 1:
        return f"BROKEN {label}: pattern found {src.count(old)} times"
    d = tempfile.mkdtemp()
    shutil.copytree(ROOT, d, dirs_exist_ok=True, ignore=shutil.ignore_patterns("__pycache__", ".git", ".venv"))
    open(os.path.join(d, rel), "w").write(src.replace(old, new))
    subprocess.run(["git", "init", "-q", d], check=True)
    subprocess.run(["git", "-C", d, "add", "-A"], check=True, capture_output=True)  # the scrub tests scan tracked files
    r = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=d,
                       env={"PYTHONPATH": os.path.join(d, "src"), "PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "")},
                       capture_output=True, text=True)
    shutil.rmtree(d)
    return ("CAUGHT " if r.returncode else "MISSED ") + label

def baseline():
    """The unmutated suite must pass, or every mutation would look caught."""
    d = tempfile.mkdtemp()
    shutil.copytree(ROOT, d, dirs_exist_ok=True, ignore=shutil.ignore_patterns("__pycache__", ".git", ".venv"))
    subprocess.run(["git", "init", "-q", d], check=True)
    subprocess.run(["git", "-C", d, "add", "-A"], check=True, capture_output=True)
    r = subprocess.run([sys.executable, "-m", "unittest", "discover", "-s", "tests", "-t", "."], cwd=d,
                       env={"PYTHONPATH": os.path.join(d, "src"), "PATH": os.environ["PATH"], "HOME": os.environ.get("HOME", "")},
                       capture_output=True, text=True)
    shutil.rmtree(d)
    return r


base = baseline()
if base.returncode != 0:
    print("BASELINE FAILS: the unmutated tests do not pass, so no mutation result means anything.\n" + base.stderr[-1500:])
    sys.exit(2)
results = [run(*m) for m in MUTATIONS]
print("\n".join(results))
bad = [r for r in results if not r.startswith("CAUGHT")]
print(f"\n{len(results) - len(bad)} of {len(results)} mutations caught")
sys.exit(1 if bad else 0)
