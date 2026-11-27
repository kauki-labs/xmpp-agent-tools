# Experiments E1 to E11 (Claude Code 2.1.323, macOS, 2026-11-26)

The evidence behind the design: what each harness and the XMPP server really do. "The client" below is the interactive
client in `src/xmpp_agent_client`. The experiment scripts were not kept, except the stdio approver behind E3 to E5
(`tests/fixtures/stub_approver_mcp.py`). One model turn cost about $0.1 to $0.3 because of the loaded tool context.

| # | Result | Evidence |
| - | ------ | -------- |
| E1 | PASS, fully | `ps eww -p <pid>` shows `CLAUDE_CODE_ENTRYPOINT=sandy`. `--debug-file` log: `User-Agent: claude-code/2.1.323 (sandy)` and `cc_entrypoint=sandy` in the billing attribution header. So claude keeps the value and reports it. |
| E2 | PASS | One stream-json process, two user lines, turn 2 answered `kiwi-42`. Init event keys include `slash_commands`, `skills`, `terminal_slash_commands`, `permissionMode`, `tools`, `mcp_servers`, `session_id`. The init event repeats on every turn. |
| E3 | PASS via `--permission-prompt-tool` only | `--permission-prompts host` alone, with or without an `initialize` control request, emitted NO `control_request`; tools were denied with "needs approval ... session non-interactive". With a stub stdio MCP `mcp__stub__approve`: request args `{tool_name, input, tool_use_id}`; reply is MCP text content holding JSON `{"behavior":"allow","updatedInput":<input>}` or `{"behavior":"deny","message":"..."}`. Allow ran the command; deny blocked it and the message reached the model. Same result inside a persistent `--input-format stream-json` process over two turns (e9_proc.py p-allow, p-deny). |
| E3b | PASS: parallel calls are serialized | Two Write calls issued in one message reached the approval tool one after the other (second CALL logged only after the first reply, 7.9s vs 1.9s with a 6s delay). One trial. Keep request ids in the chat format anyway. |
| E3c | PASS: crash fails closed | Stub killed mid-approval: the tool call failed with "Connection closed", the file was NOT written. Claude then respawned the MCP server on retry (twice per call). The approver must therefore be a thin shim that talks to the long-lived bridge and denies when the bridge is unreachable. |
| E4 | PASS | `AskUserQuestion` arrives at the same permission tool with `input.questions[]` (`question`, `header`, `options[].label/description`, `multiSelect`). Allow with unchanged input = model sees "no answer". Allow with `updatedInput = {...input, answers: {<question>: <label>}}` = model sees the pick. |
| E5 | PARTIAL, corrected | Initial matrix (the operator's user settings were active: `defaultMode: auto`, `Bash(git:*)`, `Bash(rm:*)` allowed, so the earlier risky-command probe was CONFOUNDED and is withdrawn). manual: Read auto-allowed, Write and Bash asked. acceptEdits: nothing asked for Write or `touch`. auto: nothing asked for benign ops. dontAsk: never asks, denies Write and Bash. plan: `ExitPlanMode` itself is sent to the permission tool. Retest with `--setting-sources project` (those rules excluded) against a real repo with a local bare `origin`: **auto ran `git commit --amend && git push --force origin main` and the remote ref changed; the permission tool was never called.** manual asked, and the deny held. Limits: the remote was a local bare repo; no classifier block was ever triggered, so what auto does when it blocks (deny, or fall back to the prompt tool) is UNKNOWN. |
| E6 | PASS with limits | `/cost`, `/context` run locally (0 turns, $0). `/compact` and `/clear` complete. A built-in skill with args (`/simplify say only the word ready ...`) ran. An unknown or not-loaded command goes to the model as plain text, NOT rejected. `terminal_slash_commands` = doctor, color, focus, reload-plugins. The `-p` list had 71 entries (11 with a colon) and LACKED the plugin skills and project skills that the interactive session had. So plugins and project skills must be declared for a headless run (`--plugin-dir`, working directory). |
| E7 | PASS both ways | Path in prompt (agent uses Read) and a base64 image content block in the stream-json user message both returned "Green". |
| E8 | PASS | `control_request` subtype `interrupt` ends the turn with `result` subtype `error_during_execution` (is_error true); the process stays usable and keeps context; `--resume <session_id>` in a new process keeps context. |
| E3d | PASS: no timeout at 10.5 min | Approval answered after 630 s, twice in one persistent process (turns of 633 s each); both Writes ran, no timeout or error. Longer waits and a stuck-forever approval are untested; the bridge still needs its own deny-on-timeout. |

## Consequences for the design
- Claude adapter: stream-json in/out plus an approval MCP shim (option C), not the bare control protocol.
- The shim handles `AskUserQuestion` (return `answers`), `ExitPlanMode` (plan text in input) and everything else (allow/deny). It denies if the bridge is down.
- Auto mode is not gateable from chat and may run destructive commands without a prompt. Offer `manual`, `acceptEdits`, `plan`, `dontAsk` in chat; make `auto` and `bypassPermissions` explicit per-session opt-in with a warning. The bridge must also pass `--setting-sources` deliberately, because user settings (allow rules, `defaultMode`) change what gets asked.
- The bridge validates `/name` against init `slash_commands` itself and rejects unknown names.
- Interrupt maps to `/stop`.
- "Allow for session" (approval choice 2): the bridge remembers the rule itself and auto-allows matching calls; no persisted permission update is relied on.

## E10 prep (help text only; nothing run against a model)
Runnable through `nix shell nixpkgs#<attr>`: codex 0.146.0, opencode 1.19.3, pi-coding-agent 0.75.4, plus the ACP adapters claude-agent-acp 0.36.1, claude-code-acp 0.36.1 and codex-acp 0.13.0. omp (oh-my-pi) is not in nixpkgs.
- codex: `exec --json` (JSONL), `-i` image, `-a/--ask-for-approval`, `-s/--sandbox`, `exec resume`, `app-server` (experimental; `generate-json-schema` gives the protocol), `mcp-server`.
- opencode: `run --format json`, `-f` attach, `-s` session, `--auto` (auto-approve), `serve` (headless), `acp` (native ACP server), `attach <url>`.
- pi: `--mode text|json|rpc`, `-p`, `--resume/--session/--fork`, `--tools` allowlist, extensions via `pi install`. No approval flag seen in help.
- omp (oh-my-pi): npm `@oh-my-pi/pi-coding-agent` 19.2.1 (repo can1357/oh-my-pi, bin `omp`, no install scripts listed), installed with `npm install --ignore-scripts` in its own empty directory. Needs Bun >= 1.4.4: nixpkgs bun is 1.3.13 and fails; `github:NixOS/nixpkgs/nixpkgs-unstable#bun` is 1.4.4 and runs it: `nix shell github:NixOS/nixpkgs/nixpkgs-unstable#bun -c bun node_modules/@oh-my-pi/pi-coding-agent/dist/cli.js ...`.
  Help shows: `--mode text|json|rpc|acp|rpc-ui`, `-p`, `--approval-mode always-ask|always-ask-cached|write|yolo`, `--auto-approve`, `--tools`, `--resume`, `--from-claude/--from-codex`, and an `acp` subcommand (ACP server over stdio). So omp HAS approvals and native ACP; pi's approval story still unknown (pi help shows none).

## E10 and E11 outcome, limited by credentials
Harnesses without credentials were skipped (codex, opencode, omp). pi had a credentials file but `pi -p --mode json` failed with "No API key found for the selected model", so it was skipped too. Only claude ran model turns, so approval events, auto modes and slash commands are NOT tested for codex, opencode, pi and omp.

E11 handshake probe (initialize and session/new over stdio, no model call):
| Server | initialize | session/new | Notes |
| ------ | ---------- | ----------- | ----- |
| opencode 1.19.3 `acp` (native) | OK, protocol 1, image + embeddedContext, load/resume/fork/list | OK, no login needed to open a session | modes: build (executes tools per configured permissions), more listed; picks up the operator's Claude skills as `available_commands` |
| omp 19.2.1 `acp` (native, bun 1.4.4) | OK, protocol 1, image + embeddedContext | OK | modes default, plan; auth method "agent" = local ~/.omp credentials; only model listed is apple/on-device |
| codex-acp 0.13.0 (nixpkgs) | OK, image, auth methods chatgpt / CODEX_API_KEY / OPENAI_API_KEY | error -32000 "Authentication required" | needs credentials before a session opens |
| claude-agent-acp 0.36.1 (nixpkgs, needs NIXPKGS_ALLOW_UNFREE=1 --impure; builds from source) | OK, image + embeddedContext, fork/resume/list/delete | OK | modes include `auto` = "Use a model classifier to approve/deny permission prompts", `default` = prompts for dangerous operations, acceptEdits; pulls claude-code 2.1.223 internally |
| pi | no ACP mode in help, no pi ACP adapter found in nixpkgs | - | rpc mode only |
Not yet observed for ANY ACP server: a `session/request_permission` call, because that needs a model turn. ACP is therefore confirmed as a transport that four harnesses speak, not yet as the approval path.

## E11 permission test through claude-agent-acp
- default mode: `session/request_permission` DOES arrive. Params: `toolCall{title, kind:"execute", rawInput{command,description}, toolCallId}` and `options` = allow_always, allow (allow_once), reject (reject_once). Client reply: `{"outcome":{"outcome":"selected","optionId":<id>}}`. Allow ran the command, reject blocked it (file absent). Streaming: `agent_message_chunk`, `tool_call`, `tool_call_update`, `usage_update`; stop via `stopReason: end_turn` on the `session/prompt` response.
- "allow_always" side effect: it persisted `{"permissions":{"allow":["Bash(touch probe.txt)"]}}` to `<workdir>/.claude/settings.local.json`. The bridge must either not offer allow_always or treat it as a deliberate file write; a project settings file changes later behaviour.
- auto mode (valid retest after fixing the test setup: global commit.gpgsign=true with an absent SSH key made the first repo setup fail silently): no `request_permission` was sent, the force-push did NOT happen (origin kept only `first`). Cause: the agent text says it declined because of the operator's standing instructions (never push to main), i.e. a model refusal driven by loaded CLAUDE.md, not a permission gate. Together with E5 (`-p`, user settings excluded, the push ran) this shows auto has no deterministic gate: the outcome depends on model and instructions. Treat auto as ungated.
- ACP carries approvals for claude. Not yet verified for opencode, omp, codex (no credentials).

## Server facts (Prosody 13.0.6, throwaway server, two slixmpp clients)
- MUC rooms are semi-anonymous by default: `muc_room_default_public_jids` is unset, so `whois` is `moderators`. The owner saw the participant's real JID; the participant saw `None` for the owner. With `muc_room_default_public_jids = true` both saw real JIDs. An approval design that identifies the answering person by JID needs that option (or a moderator client). Room defaults apply at room creation only.
- `mod_http_openmetrics` serves `prosody_mod_c2s__connections`, `prosody_mod_muc__live_room` and room hit and miss counters (note the double underscore); with `openmetrics_allow_cidr` set and an empty IP list a loopback request was refused (HTTP 403) outside the CIDR and served inside it.

## E12 to E15: ACP behaviour of claude-agent-acp 0.36.1 (2026-11-27)
One trial each, default permission mode, a real model; script: `experiments/acp/acp_lab.py`. The adapter was started with
`CLAUDE_CODE_ENTRYPOINT` set.

| # | Question | Result |
| - | -------- | ------ |
| E12 | How does a dialog (`AskUserQuestion`) reach the client? | **It does not.** The adapter disables the tool ("AskUserQuestion tool disabled this session"): no permission request and no other request to the client arrived, and the agent asked its question as plain text. |
| E13a | Prompt queueing: a second `session/prompt` while the first streams | PASS: both calls returned `end_turn`, and the second answer followed the first one's text. The adapter advertises `promptQueueing`. |
| E13b | `session/cancel` mid-turn | PASS: the pending prompt returned `stopReason: cancelled` at once, and the next prompt on the same session worked. |
| E14 | Park and resume in a new process | PASS: `session/load` (replays history, two updates) and `session/resume` (no replay) both restored the context: the agent answered `mango-7` to a word given in the first process. |
| E15 | Does a denied step stop the rest of the work? | No. Two independent steps: the first was rejected (`reject_once`) and the second allowed; the second ran (its file exists), and the agent reported the first as refused. A reject carries **no message**: the agent learns only that permission was refused. |

Consequences for the client:
- A dialog is ordinary chat. A question from the agent is a normal message and the human's reply is the next prompt. The numbered
  `AskUserQuestion` flow in `approvals.py` applies only to a stream-json fallback.
- "Blocked on a human" cannot ride on a denial. After a timeout the client denies, then queues a prompt that says so and tells the
  agent to continue with independent work (queueing works, E13a).
- Steering: queue by default, cancel for urgent messages.
- Resuming a session by id needs no summary; a summary is only needed when a different session takes over.

Limits: single short runs (the queueing turn took 3 s); cancel during a running tool call, a different working directory for
`session/load`, and long histories were not tested.

