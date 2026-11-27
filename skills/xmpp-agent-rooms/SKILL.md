---
name: xmpp-agent-rooms
description: Create chat rooms and add or remove members through the xmpp-agent-rooms CLI or MCP tools. Use when asked to make a room, give someone access to a room, or remove them. Not for sending messages.
---

# xmpp-agent-rooms

Only higher entities have this tool. It calls a rights service; it holds no XMPP credential.

## Before you use it

- Run it only when `XAT_CAPABILITIES` contains `rooms`. Without it the CLI refuses and the MCP server lists no tools.
- `XAT_RIGHTS_URL` and `XAT_RIGHTS_TOKEN` must be set. Never print, log or paste the token.

## CLI

```sh
xmpp-agent-rooms create <name>
xmpp-agent-rooms assign <room> <jid>
xmpp-agent-rooms unassign <room> <jid>
```

Exit code 0 means done. Exit code 1 prints the reason on stderr. Exit code 2 means a usage error.

## MCP tools

`xmpp-agent-rooms mcp` serves `room_create(name)`, `room_assign(room, jid)` and `room_unassign(room, jid)`. A refusal comes back as a tool error with the reason.

## Rules for the inputs

- Room name: `[a-z0-9_]{1,32}`. A name starting with `org_` is refused.
- Member: a bare JID, `<local>@<domain>`, with no resource, slash or space.
- Assign adds a member. Unassign removes one. You cannot grant admin or owner.

## When it fails

- `401` or `403`: you are not allowed. Do not retry; tell the user.
- `400` or `409` (the message names it): read the reason, fix the input once, then ask the user.
- No answer or an odd reply: treat it as failed. Do not assume the room exists.
