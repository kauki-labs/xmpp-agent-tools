# Overview

```
XMPP client (any app) <-> XMPP server <-> xmpp_agent_client <-> harness over ACP (claude, codex, opencode, pi, omp)
                                              ^
                       xmpp-agent-rooms (room tools, capability gated) -> a rights service that holds the room rights
```

| Module | Job | State |
| ------ | --- | ----- |
| `xmpp_agent_common.redact` | Remove secrets from tool arguments, describe a tool call for chat, cut to a size. Redacts first, then cuts. | built, tested |
| `xmpp_agent_client.router` | Classify a message: prompt, client command (`/stop`, `/mode`), harness command, or a rejection. Unknown names are refused because a harness would take them as plain text. | built, tested |
| `xmpp_agent_client.approvals` | Tool approvals and dialogs through chat: only an approver's real JID answers, one decision per request, timeout and shutdown deny. | built, tested |
| ACP adapter, XMPP layer, session loop | Run the harness, relay text both ways, files, history, presence. | planned |
| `xmpp_agent_mcp` | Directory and guide tools for everyone. | planned |
| `xmpp-agent-rooms` | Room creation and assignment as a CLI and an MCP server, in a separate binary that only higher entities get. It holds no XMPP credential: it calls a rights service ([contract](rights-service-api.md)) with a bearer token, and does nothing unless `XAT_CAPABILITIES` contains `rooms`. | built, tested; never run against a real rights service |

Facts every module relies on are in [experiments.md](experiments.md): the permission tool receives
`{tool_name, input, tool_use_id}` and answers `{"behavior":"allow","updatedInput":...}` or `{"behavior":"deny","message":...}`; a
dialog is answered through `updatedInput.answers`; `auto` mode cannot be gated; an unknown `/name` reaches the model as text.

`just test` runs the unit tests, `just mutate` the mutation check (`scripts/mutation-check.py`), `just scrub` the public-repo scrub.
