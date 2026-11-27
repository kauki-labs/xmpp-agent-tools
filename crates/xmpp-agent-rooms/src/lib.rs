//! Room tools for agents: create a room, assign or unassign a member.
//!
//! The binary holds no XMPP credential. It calls a rights service over HTTP with a bearer token (`docs/rights-service-api.md`). The same
//! actions are offered as CLI subcommands and as MCP tools. Pure logic (validation, request building, response mapping, config parsing)
//! takes its inputs as arguments and does no I/O, so the tests need no network and no environment changes.

pub mod api;
pub mod app;
pub mod cli;
pub mod client;
pub mod config;
pub mod errors;
pub mod mcp;
pub mod tools;
pub mod validate;
