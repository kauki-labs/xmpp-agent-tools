//! Command line: `create`, `assign`, `unassign` and `mcp`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::tools::Action;

/// Room tools for agents.
#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "xmpp-agent-rooms", version)]
pub struct Cli {
    /// TOML file with `rights_url`, `rights_token` and `timeout_secs`. Environment variables override it.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    /// Capabilities to grant, as `XAT_CAPABILITIES` would. Without `rooms` nothing runs.
    #[arg(long, global = true)]
    pub capabilities: Option<String>,
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// A subcommand.
#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    /// Create a room.
    Create {
        /// Room name, `[a-z0-9_]{1,32}`.
        name: String,
    },
    /// Make a JID a member of a room.
    Assign {
        /// Room name.
        room: String,
        /// Bare JID of the member.
        jid: String,
    },
    /// Remove a JID's membership of a room.
    Unassign {
        /// Room name.
        room: String,
        /// Bare JID of the member.
        jid: String,
    },
    /// Serve the same tools as an MCP server on stdio.
    Mcp,
}

impl Command {
    /// The action of a room subcommand, or `None` for `mcp`.
    pub fn action(&self) -> Option<Action> {
        match self {
            Self::Create { name } => Some(Action::Create { name: name.clone() }),
            Self::Assign { room, jid } => Some(Action::Assign { room: room.clone(), jid: jid.clone() }),
            Self::Unassign { room, jid } => Some(Action::Unassign { room: room.clone(), jid: jid.clone() }),
            Self::Mcp => None,
        }
    }
}

#[cfg(test)]
mod parsing {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(std::iter::once("xmpp-agent-rooms").chain(args.iter().copied()))
    }

    #[test]
    fn parse_should_build_a_create_action_when_given_a_name() {
        let cli = parse(&["create", "lab"]).expect("parses");
        assert_eq!(cli.command.action(), Some(Action::Create { name: "lab".to_owned() }));
    }

    #[test]
    fn parse_should_build_assign_and_unassign_actions_when_given_a_room_and_a_jid() {
        let assign = parse(&["assign", "lab", "alice@chat.example.org"]).expect("parses");
        assert_eq!(assign.command.action(), Some(Action::Assign { room: "lab".to_owned(), jid: "alice@chat.example.org".to_owned() }));
        let unassign = parse(&["unassign", "lab", "alice@chat.example.org"]).expect("parses");
        assert_eq!(unassign.command.action(), Some(Action::Unassign { room: "lab".to_owned(), jid: "alice@chat.example.org".to_owned() }));
    }

    #[test]
    fn parse_should_have_no_action_when_the_command_is_mcp() {
        assert_eq!(parse(&["mcp"]).expect("parses").command.action(), None);
    }

    #[test]
    fn parse_should_accept_the_global_flags_before_or_after_the_subcommand() {
        let before = parse(&["--capabilities", "rooms", "--config", "/etc/xat.toml", "mcp"]).expect("parses");
        let after = parse(&["mcp", "--capabilities", "rooms", "--config", "/etc/xat.toml"]).expect("parses");
        assert_eq!(before, after);
        assert_eq!(before.capabilities.as_deref(), Some("rooms"));
        assert_eq!(before.config, Some(PathBuf::from("/etc/xat.toml")));
    }

    #[test]
    fn parse_should_refuse_when_arguments_are_missing_or_the_command_is_unknown() {
        for args in [&[][..], &["create"], &["assign", "lab"], &["unassign"], &["delete", "lab"], &["create", "a", "b"]] {
            assert!(parse(args).is_err(), "{args:?}");
        }
    }

    #[test]
    fn parse_should_have_no_token_flag_when_the_token_must_stay_out_of_the_process_list() {
        assert!(parse(&["--token", "x", "mcp"]).is_err());
    }
}
