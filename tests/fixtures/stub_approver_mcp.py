"""Minimal stdio MCP server exposing one tool `approve` for --permission-prompt-tool.
Env: STUB_LOG (log file), STUB_DECISION (file: 'allow' | 'deny' | 'die'), STUB_DELAY (seconds before replying).
Each tools/call runs in its own thread so concurrent requests are visible in the log."""
import json, os, sys, threading, time

LOG = os.environ.get("STUB_LOG", "stub.log")
DECISION = os.environ.get("STUB_DECISION", "stub.decision")
DELAY = float(os.environ.get("STUB_DELAY", "0"))
out_lock, log_lock = threading.Lock(), threading.Lock()
T0 = time.time()

def log(msg):
    with log_lock, open(LOG, "a") as f:
        f.write(f"[{time.time() - T0:7.1f}s] {msg}\n")

def reply(id_, result=None, error=None):
    m = {"jsonrpc": "2.0", "id": id_}
    m["error" if error else "result"] = error or result
    with out_lock:
        sys.stdout.write(json.dumps(m) + "\n"); sys.stdout.flush()

def handle_call(id_, args):
    log("CALL " + json.dumps(args))
    try:
        d = open(DECISION).read().strip()
    except OSError:
        d = "deny"
    if d == "die":
        log("DIE (simulated bridge crash mid-approval)")
        os._exit(1)
    time.sleep(DELAY)
    if d == "allow" and args.get("tool_name") == "AskUserQuestion":
        inp = args.get("input", {})
        answers = {q["question"]: q["options"][1]["label"] for q in inp.get("questions", [])}
        out = {"behavior": "allow", "updatedInput": {**inp, "answers": answers}}
    elif d == "allow":
        out = {"behavior": "allow", "updatedInput": args.get("input", {})}
    else:
        out = {"behavior": "deny", "message": "Denied by the operator via chat."}
    log("DECIDE " + json.dumps(out)[:200])
    reply(id_, {"content": [{"type": "text", "text": json.dumps(out)}]})

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    msg = json.loads(line)
    method, id_ = msg.get("method"), msg.get("id")
    if method == "initialize":
        reply(id_, {"protocolVersion": msg["params"].get("protocolVersion", "2025-06-18"),
                    "capabilities": {"tools": {}}, "serverInfo": {"name": "stub", "version": "0"}})
    elif method == "tools/list":
        reply(id_, {"tools": [{"name": "approve", "description": "Approve or deny a tool call",
                               "inputSchema": {"type": "object",
                                               "properties": {"tool_name": {"type": "string"},
                                                              "input": {"type": "object"},
                                                              "tool_use_id": {"type": "string"}},
                                               "required": ["tool_name", "input"]}}]})
    elif method == "tools/call":
        threading.Thread(target=handle_call, args=(id_, msg["params"].get("arguments", {})), daemon=True).start()
    elif id_ is not None:
        reply(id_, error={"code": -32601, "message": "method not found"})
