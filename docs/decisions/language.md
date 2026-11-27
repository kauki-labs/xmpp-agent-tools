---
title: Implementation language
status: open, decided by a spike
updated: 2026-11-27
---

# Implementation language

**Context.** The core (router, redactor, approvals broker) is written in Python with the standard library only. A Rust client would be
one static binary with no interpreter to carry into VMs, and there are official Rust SDKs for ACP (`agent-client-protocol` 5.6.1) and
MCP (`rmcp` 3.12.0). The XMPP side is the open question.

**Checked (crates.io, 2026-11-27).** `tokio-xmpp` 6.0.0 has direct TLS and stream management. `xmpp-parsers` 0.23.0 has parsers for
MUC, MAM, message correction, HTTP upload, reactions, chat states and carbons, but only legacy OMEMO parsers and no OMEMO crypto.
**Not checked:** that a client built on them handles real-JID MUC, MAM paging and the upload round trip against a real server.
Python alternative: `slixmpp` 1.17.0 with `slixmpp-omemo` 2.2.0 (its source handles group chats) and the official `mcp` SDK.

**Decision.** Keep Python until experiment E16 decides: a time-boxed spike of a small `tokio-xmpp` program against a throwaway Prosody
(direct TLS, join a MUC and read real JIDs, send and receive a correction and a reaction, page MAM, upload and download a file). If it
passes, build in Rust and port the three modules, using their tests as the specification. If not, stay on Python.

**Consequences.** The pure modules take no I/O, so a port is a translation of logic and tests, not a redesign.
