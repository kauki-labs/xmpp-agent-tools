//! The real binary: `--version`, `--check-config` and the exit codes.

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_xmpp-agent");

fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(BIN)
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .output()
        .expect("spawn")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn write_config(name: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("xat-agent-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let path = dir.join("agent.toml");
    std::fs::write(&path, body).expect("write");
    path
}

const GOOD: &str = "jid = \"agent@chat.example.org\"\nallowed = [\"alice@chat.example.org\"]\n";

#[test]
fn version_should_print_the_name_and_exit_zero() {
    let out = run(&["--version"], &[]);
    assert!(out.status.success());
    assert!(
        text(&out.stdout).starts_with("xmpp-agent "),
        "{}",
        text(&out.stdout)
    );
}

#[test]
fn check_config_should_exit_zero_when_the_config_is_usable() {
    let path = write_config("good", GOOD);
    let out = run(
        &["--config", path.to_str().expect("utf8"), "--check-config"],
        &[("XAT_PASSWORD", "pw-from-env-7c1e")],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(
        !text(&out.stdout).contains("pw-from-env-7c1e")
            && !text(&out.stderr).contains("pw-from-env-7c1e")
    );
}

#[test]
fn check_config_should_exit_two_and_name_the_setting_when_one_is_missing() {
    let path = write_config("nopw", GOOD);
    let out = run(
        &["--config", path.to_str().expect("utf8"), "--check-config"],
        &[],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr).contains("password"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn check_config_should_read_the_password_from_the_file_named_in_the_config() {
    let pw = write_config("pwfile", "pw-in-file-9b2d\n");
    let body = format!("{GOOD}password_file = \"{}\"\n", pw.to_str().expect("utf8"));
    let path = write_config("withfile", &body);
    let out = run(
        &["--config", path.to_str().expect("utf8"), "--check-config"],
        &[],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

#[test]
fn check_config_should_exit_two_when_the_config_file_does_not_exist() {
    let out = run(
        &["--config", "/nonexistent/agent.toml", "--check-config"],
        &[("XAT_PASSWORD", "pw")],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(
        text(&out.stderr).contains("/nonexistent/agent.toml"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn check_config_should_exit_two_when_the_config_is_broken_and_not_echo_a_password_key() {
    let path = write_config("broken", &format!("{GOOD}password = \"hunter2-value\"\n"));
    let out = run(
        &["--config", path.to_str().expect("utf8"), "--check-config"],
        &[],
    );
    assert_eq!(out.status.code(), Some(2));
    assert!(
        !text(&out.stderr).contains("hunter2-value"),
        "{}",
        text(&out.stderr)
    );
}

#[test]
fn the_binary_should_take_everything_from_the_environment_when_there_is_no_file() {
    let env = [
        ("XAT_JID", "agent@chat.example.org"),
        ("XAT_PASSWORD", "pw"),
        ("XAT_ALLOWED", "alice@chat.example.org"),
    ];
    assert_eq!(run(&["--check-config"], &env).status.code(), Some(0));
}

#[test]
fn the_binary_should_refuse_a_password_flag() {
    let out = run(&["--password", "x", "--check-config"], &[]);
    assert_eq!(out.status.code(), Some(2), "clap's own usage error");
}
