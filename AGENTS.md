# xmpp-agent-tools agent instructions

Single source of truth for AI coding agents. Start with [README.md](README.md), then [docs/overview.md](docs/overview.md). The
evidence behind the design is [docs/experiments.md](docs/experiments.md); decisions are in [docs/decisions/](docs/decisions/).

## Rules

- Test first. `just test` runs the standard-library `unittest` suite with no network. A new behaviour starts as a failing test that
  fails for a functional reason (not an import error).
- Prove the tests can fail: `just mutate` applies one-line mutations and requires the suite to fail for each. A mutation that
  survives means a weak test; fix the test, not the mutation list. The script refuses to run when the unmutated suite fails.
- Fail closed. A timeout, a crash, a malformed or stale reply, a sender who is not an approver, an unknown result all deny.
- Never log or post secrets. Tool arguments are redacted, then cut, before they reach chat or a log.
- Pure logic stays free of I/O: the router, the redactor and the approvals broker take their clock and ids as arguments, so tests need
  no sleeps and the modules port to another language unchanged.
- This repository is public. Use placeholders (`chat.example.org`, `192.0.2.1`, `/home/user`) and never real hostnames, addresses,
  home paths or personal names. `just scrub` checks, and CI enforces it.
- Set `CLAUDE_CODE_ENTRYPOINT` explicitly on every `claude` process the client starts; its value comes from the launch config.
- Workflows: actions pinned by commit SHA with the version in a comment, permissions scoped per job, no `${{ }}` inside `run:`
  blocks (pass values through `env:`), and zizmor and actionlint clean before committing.

## Scope

This repository is the plumbing: a client for agents on XMPP and gated room tools. Roles, organization, who may address whom,
task tracking and observation of conversations belong to other components. Names and identities arrive through the launch config.
