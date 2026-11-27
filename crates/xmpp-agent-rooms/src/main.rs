//! `xmpp-agent-rooms`: the room tools as a CLI and an MCP server.

use std::process::ExitCode;

use clap::Parser;
use rmcp::ServiceExt;
use xmpp_agent_rooms::app;
use xmpp_agent_rooms::cli::{Cli, Command};
use xmpp_agent_rooms::config::Env;
use xmpp_agent_rooms::errors::{ConfigError, RoomsError};

/// The settings from the environment, read once here so the rest takes plain values.
fn read_env() -> Env {
    let var = |name: &str| std::env::var(name).ok();
    Env {
        rights_url: var("XAT_RIGHTS_URL"),
        token: var("XAT_RIGHTS_TOKEN"),
        capabilities: var("XAT_CAPABILITIES"),
    }
}

fn run(cli: &Cli) -> Result<String, RoomsError> {
    let env = read_env();
    let file = match &cli.config {
        Some(path) => Some(std::fs::read_to_string(path).map_err(|e| {
            ConfigError::File(format!("cannot read {}: {}", path.display(), e.kind()))
        })?),
        None => None,
    };
    let (flag, file) = (cli.capabilities.as_deref(), file.as_deref());
    match cli.command.action() {
        Some(action) => app::rooms(flag, &env, file)?.run(&action),
        None => serve(app::mcp_server(flag, &env, file)?),
    }
}

/// Serve MCP on stdio until the client closes it. Stdout carries the protocol only; nothing else writes to it.
fn serve(server: app::Server) -> Result<String, RoomsError> {
    let failed = |e: &dyn std::fmt::Display| {
        RoomsError::Config(ConfigError::File(format!("mcp server stopped: {e}")))
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| failed(&e))?;
    runtime.block_on(async {
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|e| failed(&e))?;
        running.waiting().await.map_err(|e| failed(&e))?;
        Ok(String::new())
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let is_mcp = matches!(cli.command, Command::Mcp);
    match run(&cli) {
        Ok(line) => {
            if !is_mcp {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}
