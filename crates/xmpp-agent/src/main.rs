//! `xmpp-agent`: connect a human on XMPP to a coding agent over ACP.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use tokio::sync::mpsc;
use xmpp_agent::chat::Chat;
use xmpp_agent::config::{Env, Settings};
use xmpp_agent::log::{FileLog, Log, LogEvent, NoLog};
use xmpp_agent::session::Session;
use xmpp_agent_xmpp::{Config as XmppConfig, Event, XmppClient};

/// Connect a human on XMPP to a coding agent over ACP (DM mode).
#[derive(Debug, Parser)]
#[command(name = "xmpp-agent", version)]
struct Args {
    /// TOML config file. Environment variables override it.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Validate the config and exit: 0 when it is usable, 2 when it is not.
    #[arg(long)]
    check_config: bool,
}

/// Exit code for a config that cannot be used.
const EXIT_CONFIG: u8 = 2;

fn read_env() -> Env {
    let var = |name: &str| std::env::var(name).ok();
    Env {
        jid: var("XAT_JID"),
        password: var("XAT_PASSWORD"),
        password_file: var("XAT_PASSWORD_FILE"),
        allowed: var("XAT_ALLOWED"),
        approvers: var("XAT_APPROVERS"),
        cwd: var("XAT_CWD"),
        approval_timeout_secs: var("XAT_APPROVAL_TIMEOUT_SECS"),
        log_path: var("XAT_LOG_PATH"),
    }
}

fn read_password_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e.kind()))
}

fn load_settings(args: &Args) -> Result<Settings, String> {
    let file = match &args.config {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {}: {}", path.display(), e.kind()))?,
        ),
        None => None,
    };
    Settings::load(file.as_deref(), &read_env(), &read_password_file).map_err(|e| e.to_string())
}

/// Ctrl-C, or SIGTERM where there is one (a service manager stops a service with it).
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

async fn serve<C: Chat, L: Log>(
    settings: Settings,
    chat: C,
    log: L,
    mut events: mpsc::Receiver<Event>,
) -> Result<(), String> {
    let mut session = Session::new(settings, chat, log);
    session.start().await.map_err(|e| e.to_string())?;
    let outcome = tokio::select! {
        outcome = session.run(&mut events) => outcome.map_err(|e| e.to_string()),
        () = shutdown_signal() => Ok(()),
    };
    session.shutdown().await;
    outcome
}

async fn run(settings: Settings) -> Result<(), String> {
    let server = settings.server.clone();
    let config = XmppConfig {
        jid: settings.jid.clone(),
        password: settings.password.expose().to_owned(),
        server,
    };
    let (client, events) = XmppClient::connect(config)
        .await
        .map_err(|e| e.to_string())?;
    let outcome = match settings.log_path.clone() {
        Some(path) => serve(settings, client.clone(), FileLog::new(&path), events).await,
        None => serve(settings, client.clone(), NoLog, events).await,
    };
    client.disconnect().await;
    outcome
}

fn main() -> ExitCode {
    let args = Args::parse();
    let settings = match load_settings(&args) {
        Ok(settings) => settings,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };
    if args.check_config {
        if !settings.cwd.is_dir() {
            eprintln!("error: cwd: {} is not a directory", settings.cwd.display());
            return ExitCode::from(EXIT_CONFIG);
        }
        println!("config ok");
        return ExitCode::SUCCESS;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("error: cannot start the runtime: {error}");
            return ExitCode::from(1);
        }
    };
    match runtime.block_on(run(settings)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(1)
        }
    }
}

#[allow(dead_code)]
fn _log_event_is_used(_: LogEvent) {}
