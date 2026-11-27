"""E12 to E15: how claude-agent-acp behaves for dialogs, queueing, cancel, load/resume and a denied step.
usage: python3 experiments/acp/acp_lab.py <e12|e13a|e13b|e14|e15>
Needs nix with `nixpkgs#claude-agent-acp` (NIXPKGS_ALLOW_UNFREE=1, --impure) and a logged-in Claude Code; each run costs a few cents.
Scratch directories (work-*) are created next to this file and ignored by git."""
import json, os, queue, shutil, subprocess, sys, threading, time

BASE = os.path.dirname(os.path.abspath(__file__))
CMD = ["nix", "shell", "--impure", "nixpkgs#claude-agent-acp", "-c", "claude-agent-acp"]


class Agent:
    def __init__(self, work, policy=None):
        self.work, self.policy = work, policy
        self.p = subprocess.Popen(CMD, cwd=work, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1,
                                  env=dict(os.environ, NIXPKGS_ALLOW_UNFREE="1", CLAUDE_CODE_ENTRYPOINT="sandy"))
        self.log = []          # (t, kind, method/summary)
        self.responses = {}
        self.cond = threading.Condition()
        self.chunks = []       # (t, text)
        self.perms = []        # full permission requests
        self.next_id = 1
        self.t0 = time.time()
        threading.Thread(target=self._read, daemon=True).start()

    def _t(self):
        return round(time.time() - self.t0, 1)

    def _read(self):
        for line in self.p.stdout:
            try:
                m = json.loads(line)
            except ValueError:
                continue
            if "method" in m and "id" in m:                       # request from the agent
                self.log.append((self._t(), "request", m["method"]))
                result = None
                if m["method"] == "session/request_permission":
                    self.perms.append(m["params"])
                    result = self.policy(m["params"]) if self.policy else {"outcome": {"outcome": "cancelled"}}
                self._send({"jsonrpc": "2.0", "id": m["id"], "result": result if result is not None else {}})
            elif "method" in m:                                   # notification
                u = m["params"].get("update", {}) if m["method"] == "session/update" else {}
                kind = u.get("sessionUpdate", "")
                self.log.append((self._t(), "notify", m["method"] + ("/" + kind if kind else "")))
                if kind == "agent_message_chunk":
                    self.chunks.append((self._t(), (u.get("content") or {}).get("text", "")))
                with self.cond:
                    self.cond.notify_all()
            else:                                                 # response
                with self.cond:
                    self.responses[m.get("id")] = m
                    self.cond.notify_all()

    def _send(self, obj):
        self.p.stdin.write(json.dumps(obj) + "\n"); self.p.stdin.flush()

    def call(self, method, params, wait=True, timeout=180):
        i = self.next_id; self.next_id += 1
        self._send({"jsonrpc": "2.0", "id": i, "method": method, "params": params})
        return self.wait(i, timeout) if wait else i

    def notify(self, method, params):
        self._send({"jsonrpc": "2.0", "method": method, "params": params})

    def wait(self, i, timeout=180):
        end = time.time() + timeout
        with self.cond:
            while i not in self.responses and time.time() < end:
                self.cond.wait(1)
        return self.responses.get(i, {"timeout": True})

    def wait_chunk(self, timeout=90):
        end = time.time() + timeout
        with self.cond:
            while not self.chunks and time.time() < end:
                self.cond.wait(1)
        return bool(self.chunks)

    def text(self):
        return "".join(t for _, t in self.chunks)

    def close(self):
        self.p.kill()


def allow(params):
    opts = params.get("options", [])
    pick = next((o for o in opts if o.get("kind") == "allow_once"), opts[0] if opts else None)
    return {"outcome": {"outcome": "selected", "optionId": pick["optionId"]}} if pick else {"outcome": {"outcome": "cancelled"}}


def start(label, policy=None, mode="default"):
    work = os.path.join(BASE, "work-" + label)
    shutil.rmtree(work, ignore_errors=True); os.makedirs(work)
    a = Agent(work, policy)
    a.call("initialize", {"protocolVersion": 1, "clientCapabilities": {}, "clientInfo": {"name": "acp-lab", "version": "0"}}, timeout=90)
    r = a.call("session/new", {"cwd": work, "mcpServers": []}, timeout=90)
    sid = r["result"]["sessionId"]
    a.call("session/set_mode", {"sessionId": sid, "modeId": mode}, timeout=30)
    return a, sid, work


def prompt(a, sid, text, wait=True, timeout=180):
    return a.call("session/prompt", {"sessionId": sid, "prompt": [{"type": "text", "text": text}]}, wait=wait, timeout=timeout)


def methods(a):
    seen = {}
    for _, kind, m in a.log:
        seen[(kind, m)] = seen.get((kind, m), 0) + 1
    return {f"{k}:{m}": n for (k, m), n in sorted(seen.items())}


which = sys.argv[1]

if which == "e12":
    a, sid, work = start("e12", allow)
    r = prompt(a, sid, "Use the AskUserQuestion tool to ask me whether to use red or blue (two options, labels Red and Blue), then tell me which one I picked.")
    print("stopReason:", r.get("result", r).get("stopReason") if "result" in r else r)
    print("methods seen:", json.dumps(methods(a), indent=1))
    print("permission requests:", len(a.perms))
    for p in a.perms:
        print(json.dumps({"toolCall": {k: p.get("toolCall", {}).get(k) for k in ("title", "kind", "rawInput")}, "options": p.get("options")}, indent=1)[:1800])
    print("agent text:", a.text()[:400])
    a.close()

elif which == "e13a":
    a, sid, work = start("e13a", allow)
    t0 = time.time()
    i1 = prompt(a, sid, "Write the numbers 1 to 120, one per line, and nothing else.", wait=False)
    ok = a.wait_chunk(90)
    t_first = round(time.time() - t0, 1)
    i2 = prompt(a, sid, "Reply with only the word: pineapple", wait=False)
    r1 = a.wait(i1, 180); t1 = round(time.time() - t0, 1)
    r2 = a.wait(i2, 180); t2 = round(time.time() - t0, 1)
    print(f"first chunk at {t_first}s (seen={ok}); prompt1 done at {t1}s stop={r1.get('result', {}).get('stopReason')}; prompt2 done at {t2}s stop={r2.get('result', {}).get('stopReason')}")
    txt = a.text()
    print("pineapple appears after the numbers:", "pineapple" in txt and txt.rfind("pineapple") > txt.find("100"))
    print("tail of text:", repr(txt[-60:]))
    print("methods:", json.dumps(methods(a)))
    a.close()

elif which == "e13b":
    a, sid, work = start("e13b", allow)
    t0 = time.time()
    i1 = prompt(a, sid, "Write the numbers 1 to 400, one per line, and nothing else.", wait=False)
    a.wait_chunk(90)
    a.notify("session/cancel", {"sessionId": sid})
    t_cancel = round(time.time() - t0, 1)
    r1 = a.wait(i1, 120)
    print(f"cancel sent at {t_cancel}s; prompt returned at {round(time.time() - t0, 1)}s: {json.dumps(r1.get('result', r1))[:200]}")
    r2 = prompt(a, sid, "Reply with only the word: ok", timeout=120)
    print("next prompt after cancel:", json.dumps(r2.get("result", r2))[:200], "| text tail:", repr(a.text()[-30:]))
    a.close()

elif which == "e14":
    a, sid, work = start("e14", allow)
    prompt(a, sid, "Remember the word mango-7. Reply with only: ok", timeout=120)
    a.close()
    # a new process, the same working directory
    for method in ("session/load", "session/resume"):
        a2 = Agent(work, allow)
        init = a2.call("initialize", {"protocolVersion": 1, "clientCapabilities": {}, "clientInfo": {"name": "acp-lab", "version": "0"}}, timeout=90)
        caps = init["result"]["agentCapabilities"]
        r = a2.call(method, {"sessionId": sid, "cwd": work, "mcpServers": []}, timeout=90)
        replayed = sum(1 for _, k, m in a2.log if k == "notify" and m.startswith("session/update"))
        print(f"{method}: response keys {sorted(r.get('result', r).keys()) if isinstance(r.get('result', r), dict) else r}; replayed updates: {replayed}")
        a2.chunks.clear()
        r2 = prompt(a2, sid, "What word did I ask you to remember? Reply with only that word.", timeout=120)
        print(f"  after {method}: stop={r2.get('result', {}).get('stopReason')} answer={a2.text()[:60]!r}")
        a2.close()

elif which == "e15":
    def policy(params):
        title = (params.get("toolCall") or {}).get("title", "")
        opts = params.get("options", [])
        pick = next((o for o in opts if o.get("kind") == ("reject_once" if "touch" in title else "allow_once")), None)
        return {"outcome": {"outcome": "selected", "optionId": pick["optionId"]}} if pick else {"outcome": {"outcome": "cancelled"}}
    a, sid, work = start("e15", policy)
    r = prompt(a, sid, "Do two independent things and report each in one line: (1) run this exact command with the Bash tool: touch a.txt (2) create the file b.txt containing hi with the Write tool.", timeout=240)
    print("stopReason:", r.get("result", r).get("stopReason") if "result" in r else r)
    print("files:", sorted(f for f in os.listdir(work) if f.endswith(".txt")))
    print("permission requests:", [(p.get("toolCall") or {}).get("title") for p in a.perms])
    print("agent text:", a.text()[:500])
    a.close()
