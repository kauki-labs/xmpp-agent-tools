# Rights service HTTP API

The rights service holds the XMPP admin credential and the rule for who may manage rooms. `xmpp-agent-rooms` calls it over HTTP and holds
no XMPP credential. The service is a separate piece and is not built here; this page is the contract both sides test against.

## Authentication

Every request carries `Authorization: Bearer <token>`. The token is bound to one caller JID. The service answers `401` when the token is
missing, malformed or unknown, and `403` when the caller is not a higher entity. The client never prints the token.

## Validation

The client checks every input before it sends a request, with the same rules as the provisioner's `room-create` and `room-assign`. The
service checks them again and answers `400` for a violation.

| Input | Rule |
| ----- | ---- |
| room name | matches `[a-z0-9_]{1,32}` over the whole string; a name starting with `org_` is refused |
| member JID | bare: one `@`, a non-empty local part, a non-empty domain, no `/`, no whitespace |
| affiliation | `member` or `none` only; never `admin` or `owner` |

## `POST /v1/rooms`

Create a room, or report that it exists.

Request body: `{"name": "<name>"}`.

Response `200`: `{"room": "<name>@<muc host>", "created": true}`. `created` is `false` when the room already existed.

## `POST /v1/rooms/{name}/members`

Set the affiliation of one member in a room.

Request body: `{"jid": "<bare jid>", "affiliation": "member"}`. Use `"none"` to remove the member.

Response `200`: `{}`.

## Errors

An error response has a JSON body `{"error": "<text>"}`. The client shows the text only after it removes secrets from it.

| Status | Meaning | Client result |
| ------ | ------- | ------------- |
| `400` | invalid name or JID | `invalid` with the service text |
| `401` | bad or missing token | `unauthorized` |
| `403` | caller is not a higher entity | `forbidden` |
| `409` | conflict | `conflict` with the service text |
| `5xx` | service failure | `server` with the status |
| any other status, a malformed body, a timeout, a connection failure | unknown outcome | an error; never success |

A `200` whose body does not match the shape above is an error. The client fails closed.

## Client settings

| Setting | Source | Default |
| ------- | ------ | ------- |
| rights service base URL | env `XAT_RIGHTS_URL`, or `rights_url` in the `--config` file | none, required |
| bearer token | env `XAT_RIGHTS_TOKEN`, or `rights_token` in the `--config` file | none, required |
| request timeout | `timeout_secs` in the `--config` file | 10 |
| capabilities | env `XAT_CAPABILITIES`, a comma or space separated list, or `--capabilities` | none; `rooms` is required |

An environment variable overrides the same setting in the file.
