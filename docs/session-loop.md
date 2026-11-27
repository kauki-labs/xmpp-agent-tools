# The session loop (DM mode)

`xmpp-agent` connects a human on XMPP to one coding agent over ACP. Rooms, files, edits, reactions, history, mode switching and OMEMO
are out of scope for this version.

## Run

```sh
xmpp-agent --config agent.toml            # run
xmpp-agent --config agent.toml --check-config   # validate: exit 0 usable, 2 not
xmpp-agent --version
```

Exit codes: 0 clean stop, 1 a runtime failure (login refused, connection ended for good, the agent did not start), 2 a config that
cannot be used.

## Config

A TOML file; an environment variable overrides the same key. An unknown key is an error, so a password written into the file is
refused. The password comes from `XAT_PASSWORD`, or from the file named by `password_file` / `XAT_PASSWORD_FILE`; there is no flag.

| Key | Env | Default |
| --- | --- | ------- |
| `jid` (bare) | `XAT_JID` | required |
| `password_file` | `XAT_PASSWORD`, `XAT_PASSWORD_FILE` | required |
| `server_host`, `server_port` | | SRV lookup of the JID's domain; port 5223 when only a host is set |
| `agent_command`, `agent_args` | | `claude-agent-acp`, none |
| `entrypoint` | | `sandy`: the value of `CLAUDE_CODE_ENTRYPOINT` in the agent's environment |
| `cwd` | `XAT_CWD` | `.` |
| `allowed` (bare JIDs) | `XAT_ALLOWED` (comma or space separated) | required |
| `approvers` (bare JIDs) | `XAT_APPROVERS` | the allowed JIDs |
| `approval_timeout_secs` | `XAT_APPROVAL_TIMEOUT_SECS` | 600, from 1 to 86400 |
| `log_path` | `XAT_LOG_PATH` | no log |

## Behaviour

- A message from a JID that is not allowed is ignored and logged. From an allowed JID it goes through the router: a prompt goes to
  the agent, `/stop` cancels the turn, `/new` starts a fresh session, `/status` and `/help` answer locally, a command the agent lists
  goes to it as text, anything else gets a one-line reply.
- The agent's text arrives as one message per turn, split at 3000 characters on a line boundary, plus one redacted line per tool call.
  Tool output and thoughts are never sent. A typing indication runs while the agent works.
- A permission request goes to the approvers with numbered options. Only an approver's real bare JID answers. A timeout denies it, and
  a queued prompt tells the agent the step is blocked on a human so it can do independent work (docs/experiments.md E15). Answer 4 with a
  note passes the note to the agent.
- Answer 2 ("allow this exact call again") behaves as allow once: the ACP permission event carries no raw input, so the loop cannot
  tell that a later call is the same one.
- A dead agent is reported once; prompts are ignored until `/new`. A reconnect of the XMPP layer is silent.
- The operational log is one JSON object per line: starts, stops, approvals asked and decided, errors. It holds no message body and
  no secret.
