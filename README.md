# xmpp-agent-tools

Tools for agents that talk over XMPP.

XMPP is a coordination channel between people and agents. These tools give an agent a place on it: an interactive client that wraps
a coding-agent harness so a human can work with it from any XMPP app, and, separately, a small set of privileged tools for creating
and assigning rooms that only higher entities receive.

**Status: early.** The first modules exist and are tested; the XMPP and harness layers are not built yet.

| Package | What it holds | State |
| ------- | ------------- | ----- |
| `xmpp_agent_common` | `redact`: secrets and long values never reach chat | built |
| `xmpp_agent_client` | `router` (chat message to prompt or command), `approvals` (tool approvals and dialogs answered in chat) | built |
| `xmpp_agent_client` | ACP harness adapter, XMPP layer, session loop | planned |
| `xmpp-agent` | the session loop (DM mode): a human on XMPP and one agent over ACP | built, tested |
| `xmpp_agent_mcp` | room tools behind a capability gate, directory and guide tools | planned |

## Develop

```sh
just test      # unit tests, standard library only
just coverage  # branch coverage, floor 90%
just mutate    # apply one-line mutations; every one must fail the tests
just scrub     # no internal names, addresses or home paths in this public repository
```

No dependency is needed to run the tests. Python 3.11 or newer.

## How it is checked

Every pull request runs `.github/workflows/pr.yaml` (see [docs/ci.md](docs/ci.md)). Its `merge-gate` job is the only required check: it is green when every other
job is success or skipped. The design facts come from experiments, recorded in [docs/experiments.md](docs/experiments.md).
See [AGENTS.md](AGENTS.md) for the rules the code follows.

## License

MIT, see [LICENSE](LICENSE).
