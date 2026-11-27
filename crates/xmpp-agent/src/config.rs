//! Settings: a TOML file with environment overrides, validated once at start.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use xmpp_agent_acp::SpawnConfig;

/// Default time an approval waits for an answer, in seconds.
pub const DEFAULT_APPROVAL_TIMEOUT_SECS: u64 = 600;

/// Longest approval timeout accepted, in seconds (one day).
pub const MAX_APPROVAL_TIMEOUT_SECS: u64 = 86_400;

/// Environment variables with a secret. The agent's process does not get them: it runs under `env -u`.
pub const SECRET_ENV: [&str; 2] = ["XAT_PASSWORD", "XAT_PASSWORD_FILE"];

/// Longest start timeout accepted, in seconds.
pub const MAX_AGENT_START_TIMEOUT_SECS: u64 = 600;

/// Default time the agent has to open a session, in seconds.
pub const DEFAULT_AGENT_START_TIMEOUT_SECS: u64 = 60;

/// Port of the direct-TLS XMPP endpoint when a server host is given without a port.
pub const DEFAULT_SERVER_PORT: u16 = 5223;

/// Value of `CLAUDE_CODE_ENTRYPOINT` for the agent process when the file sets none.
pub const DEFAULT_ENTRYPOINT: &str = "sandy";

/// A secret. `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Wrap a secret.
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The value, for the login only.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

/// The environment overrides, read once by the caller. A set but empty value counts as unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// `XAT_JID`.
    pub jid: Option<String>,
    /// `XAT_PASSWORD`.
    pub password: Option<String>,
    /// `XAT_PASSWORD_FILE`.
    pub password_file: Option<String>,
    /// `XAT_ALLOWED`: bare JIDs separated by commas or whitespace.
    pub allowed: Option<String>,
    /// `XAT_APPROVERS`: bare JIDs separated by commas or whitespace.
    pub approvers: Option<String>,
    /// `XAT_CWD`.
    pub cwd: Option<String>,
    /// `XAT_APPROVAL_TIMEOUT_SECS`.
    pub approval_timeout_secs: Option<String>,
    /// `XAT_LOG_PATH`.
    pub log_path: Option<String>,
}

/// Why the settings are unusable. No variant holds a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// The file is not valid TOML for these keys.
    Parse(String),
    /// A required setting is absent.
    Missing(&'static str),
    /// A setting has a bad value.
    Invalid {
        /// The setting.
        field: &'static str,
        /// What is wrong.
        reason: String,
    },
    /// The password file cannot be read.
    PasswordFile(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(text) => write!(f, "config file: {text}"),
            Self::Missing(name) => write!(f, "missing setting: {name}"),
            Self::Invalid { field, reason } => write!(f, "{field}: {reason}"),
            Self::PasswordFile(text) => write!(f, "password file: {text}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// The validated settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The agent's bare JID.
    pub jid: String,
    /// The password of that account.
    pub password: Secret,
    /// Host and port of a direct-TLS server; `None` looks up the SRV record of the JID's domain.
    pub server: Option<(String, u16)>,
    /// The ACP agent command.
    pub agent_command: String,
    /// Its arguments.
    pub agent_args: Vec<String>,
    /// Value of `CLAUDE_CODE_ENTRYPOINT` in the agent's environment.
    pub entrypoint: String,
    /// The agent's working directory.
    pub cwd: PathBuf,
    /// Bare, lowercase JIDs that may talk to the agent.
    pub allowed: Vec<String>,
    /// Bare, lowercase JIDs whose answers decide approvals.
    pub approvers: Vec<String>,
    /// Time an approval waits before it is denied.
    pub approval_timeout: Duration,
    /// Time the agent has to open a session before the start is given up.
    pub agent_start_timeout: Duration,
    /// Where the operational JSONL log goes; `None` writes none.
    pub log_path: Option<PathBuf>,
}

/// The keys of the config file. Unknown keys are an error, so a password written into the file is refused, not ignored.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSettings {
    jid: Option<String>,
    password_file: Option<String>,
    server_host: Option<String>,
    server_port: Option<u16>,
    agent_command: Option<String>,
    agent_args: Option<Vec<String>>,
    entrypoint: Option<String>,
    cwd: Option<String>,
    allowed: Option<Vec<String>>,
    approvers: Option<Vec<String>>,
    approval_timeout_secs: Option<u64>,
    agent_start_timeout_secs: Option<u64>,
    log_path: Option<String>,
}

/// The agent command when the file names none.
const DEFAULT_AGENT_COMMAND: &str = "claude-agent-acp";

fn invalid(field: &'static str, reason: &str) -> ConfigError {
    ConfigError::Invalid {
        field,
        reason: reason.to_owned(),
    }
}

/// An error text that names the line and never quotes the file: the parser's message can repeat a value.
fn file_error(text: &str, error: &toml::de::Error) -> ConfigError {
    let line = error.span().map_or(0, |span| {
        text.get(..span.start)
            .map_or(0, |head| head.matches('\n').count() + 1)
    });
    ConfigError::Parse(format!("not valid TOML for these keys (line {line})"))
}

fn split_list(list: &str) -> Vec<String> {
    list.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A bare JID in lowercase: `<local>@<domain>`, one `@`, no resource, no whitespace or control character.
fn bare_jid(field: &'static str, jid: &str) -> Result<String, ConfigError> {
    let forbidden = |c: char| c == '/' || c.is_whitespace() || c.is_control();
    match jid.split_once('@') {
        Some((local, domain))
            if !local.is_empty()
                && !domain.is_empty()
                && !domain.contains('@')
                && !jid.contains(forbidden) =>
        {
            Ok(jid.to_lowercase())
        }
        _ => Err(invalid(field, "must be a bare JID: <local>@<domain>")),
    }
}

fn bare_list(field: &'static str, jids: &[String]) -> Result<Vec<String>, ConfigError> {
    let mut out: Vec<String> = Vec::new();
    for jid in jids {
        let jid = bare_jid(field, jid)?;
        if !out.contains(&jid) {
            out.push(jid);
        }
    }
    Ok(out)
}

impl Settings {
    /// Parse the TOML text of the config file (if any), apply the environment over it, and validate.
    ///
    /// `read_password_file` reads a password file; it is called only when no password is set in the environment.
    pub fn load(
        file: Option<&str>,
        env: &Env,
        read_password_file: &dyn Fn(&Path) -> Result<String, String>,
    ) -> Result<Self, ConfigError> {
        let raw = match file {
            Some(text) => toml::from_str::<FileSettings>(text).map_err(|e| file_error(text, &e))?,
            None => FileSettings::default(),
        };
        let set = |value: &Option<String>| {
            value
                .as_deref()
                .filter(|v| !v.trim().is_empty())
                .map(str::to_owned)
        };

        let jid = set(&env.jid)
            .or_else(|| raw.jid.clone())
            .ok_or(ConfigError::Missing("jid"))?;
        let jid = bare_jid("jid", &jid)?;

        let allowed = match set(&env.allowed) {
            Some(list) => split_list(&list),
            None => raw.allowed.clone().unwrap_or_default(),
        };
        if allowed.is_empty() {
            return Err(ConfigError::Missing("allowed"));
        }
        let allowed = bare_list("allowed", &allowed)?;
        let approvers = match (set(&env.approvers), &raw.approvers) {
            (Some(list), _) => bare_list("approvers", &split_list(&list))?,
            (None, Some(list)) => bare_list("approvers", list)?,
            (None, None) => allowed.clone(),
        };
        if approvers.is_empty() {
            return Err(invalid("approvers", "must not be empty"));
        }

        let password = match set(&env.password) {
            Some(password) => password,
            None => {
                let path = set(&env.password_file)
                    .or_else(|| raw.password_file.clone())
                    .ok_or(ConfigError::Missing("password"))?;
                let text =
                    read_password_file(Path::new(&path)).map_err(ConfigError::PasswordFile)?;
                let password = text.trim_end_matches(['\r', '\n']);
                if password.is_empty() {
                    return Err(ConfigError::Missing("password"));
                }
                password.to_owned()
            }
        };

        let timeout_secs = match set(&env.approval_timeout_secs) {
            Some(text) => text.trim().parse::<u64>().map_err(|_| {
                invalid("approval_timeout_secs", "must be a whole number of seconds")
            })?,
            None => raw
                .approval_timeout_secs
                .unwrap_or(DEFAULT_APPROVAL_TIMEOUT_SECS),
        };
        if !(1..=MAX_APPROVAL_TIMEOUT_SECS).contains(&timeout_secs) {
            return Err(invalid(
                "approval_timeout_secs",
                &format!("must be between 1 and {MAX_APPROVAL_TIMEOUT_SECS}"),
            ));
        }

        let start_timeout_secs = raw
            .agent_start_timeout_secs
            .unwrap_or(DEFAULT_AGENT_START_TIMEOUT_SECS);
        if !(1..=MAX_AGENT_START_TIMEOUT_SECS).contains(&start_timeout_secs) {
            return Err(invalid(
                "agent_start_timeout_secs",
                &format!("must be between 1 and {MAX_AGENT_START_TIMEOUT_SECS}"),
            ));
        }

        let host = raw.server_host.clone().filter(|h| !h.trim().is_empty());
        let server = match (host, raw.server_port) {
            (Some(_), Some(0)) | (None, Some(0)) => {
                return Err(invalid("server_port", "must be between 1 and 65535"));
            }
            (Some(host), port) => Some((host, port.unwrap_or(DEFAULT_SERVER_PORT))),
            (None, Some(_)) => return Err(invalid("server_port", "needs server_host")),
            (None, None) => None,
        };

        let agent_command = raw
            .agent_command
            .clone()
            .unwrap_or_else(|| DEFAULT_AGENT_COMMAND.to_owned());
        if agent_command.trim().is_empty() {
            return Err(invalid("agent_command", "must not be empty"));
        }

        Ok(Self {
            jid,
            password: Secret::new(password),
            server,
            agent_command,
            agent_args: raw.agent_args.clone().unwrap_or_default(),
            entrypoint: raw
                .entrypoint
                .clone()
                .filter(|e| !e.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_ENTRYPOINT.to_owned()),
            cwd: PathBuf::from(
                set(&env.cwd)
                    .or_else(|| raw.cwd.clone())
                    .unwrap_or_else(|| ".".to_owned()),
            ),
            allowed,
            approvers,
            approval_timeout: Duration::from_secs(timeout_secs),
            agent_start_timeout: Duration::from_secs(start_timeout_secs),
            log_path: set(&env.log_path)
                .or_else(|| raw.log_path.clone())
                .map(PathBuf::from),
        })
    }

    /// How to start the agent.
    pub fn spawn_config(&self) -> SpawnConfig {
        let mut args: Vec<String> = SECRET_ENV
            .iter()
            .flat_map(|name| ["-u".to_owned(), (*name).to_owned()])
            .collect();
        args.push(self.agent_command.clone());
        args.extend(self.agent_args.iter().cloned());
        SpawnConfig {
            command: "env".to_owned(),
            args,
            env: vec![("CLAUDE_CODE_ENTRYPOINT".to_owned(), self.entrypoint.clone())],
            cwd: self.cwd.clone(),
        }
    }
}

#[cfg(test)]
mod loading {
    use rstest::rstest;

    use super::*;

    const MINIMAL: &str = "jid = \"agent@chat.example.org\"\npassword_file = \"/run/secrets/xat\"\nallowed = [\"alice@chat.example.org\"]\n";

    fn reader(text: &str) -> impl Fn(&Path) -> Result<String, String> + '_ {
        move |_| Ok(text.to_owned())
    }

    fn load(file: &str, env: &Env) -> Result<Settings, ConfigError> {
        Settings::load(Some(file), env, &reader("s3cret-pw\n"))
    }

    #[test]
    fn load_should_fill_the_defaults_when_the_file_has_only_the_required_keys() {
        let s = load(MINIMAL, &Env::default()).expect("loads");
        assert_eq!(s.jid, "agent@chat.example.org");
        assert_eq!(s.password.expose(), "s3cret-pw");
        assert_eq!(s.server, None);
        assert_eq!(s.agent_command, "claude-agent-acp");
        assert!(s.agent_args.is_empty());
        assert_eq!(s.entrypoint, "sandy");
        assert_eq!(s.cwd, PathBuf::from("."));
        assert_eq!(s.allowed, ["alice@chat.example.org"]);
        assert_eq!(s.approvers, ["alice@chat.example.org"]);
        assert_eq!(s.approval_timeout, Duration::from_secs(600));
        assert_eq!(s.log_path, None);
    }

    #[test]
    fn load_should_read_every_key_when_the_file_sets_them() {
        let file = r#"
jid = "agent@chat.example.org"
password_file = "/run/secrets/xat"
server_host = "xmpp.example.org"
server_port = 5223
agent_command = "my-agent"
agent_args = ["--acp", "--quiet"]
entrypoint = "lab"
cwd = "/home/user/work"
allowed = ["Alice@Chat.Example.org", "bob@chat.example.org"]
approvers = ["bob@chat.example.org"]
approval_timeout_secs = 30
log_path = "/var/log/xat.jsonl"
"#;
        let s = load(file, &Env::default()).expect("loads");
        assert_eq!(s.server, Some(("xmpp.example.org".to_owned(), 5223)));
        assert_eq!(
            (s.agent_command.as_str(), s.agent_args.as_slice()),
            (
                "my-agent",
                ["--acp".to_owned(), "--quiet".to_owned()].as_slice()
            )
        );
        assert_eq!(s.entrypoint, "lab");
        assert_eq!(s.cwd, PathBuf::from("/home/user/work"));
        assert_eq!(
            s.allowed,
            ["alice@chat.example.org", "bob@chat.example.org"]
        );
        assert_eq!(s.approvers, ["bob@chat.example.org"]);
        assert_eq!(s.approval_timeout, Duration::from_secs(30));
        assert_eq!(s.log_path, Some(PathBuf::from("/var/log/xat.jsonl")));
    }

    #[test]
    fn load_should_prefer_the_environment_when_it_sets_the_same_value_as_the_file() {
        let env = Env {
            jid: Some("other@chat.example.org".to_owned()),
            allowed: Some(
                "carol@chat.example.org, dave@chat.example.org erin@chat.example.org".to_owned(),
            ),
            approvers: Some("dave@chat.example.org".to_owned()),
            cwd: Some("/home/user/env".to_owned()),
            approval_timeout_secs: Some("45".to_owned()),
            log_path: Some("/tmp/env.jsonl".to_owned()),
            ..Env::default()
        };
        let s = load(MINIMAL, &env).expect("loads");
        assert_eq!(s.jid, "other@chat.example.org");
        assert_eq!(
            s.allowed,
            [
                "carol@chat.example.org",
                "dave@chat.example.org",
                "erin@chat.example.org"
            ]
        );
        assert_eq!(s.approvers, ["dave@chat.example.org"]);
        assert_eq!(s.cwd, PathBuf::from("/home/user/env"));
        assert_eq!(s.approval_timeout, Duration::from_secs(45));
        assert_eq!(s.log_path, Some(PathBuf::from("/tmp/env.jsonl")));
    }

    #[test]
    fn load_should_work_without_a_file_when_the_environment_has_everything() {
        let env = Env {
            jid: Some("agent@chat.example.org".to_owned()),
            password: Some("pw-from-env".to_owned()),
            allowed: Some("alice@chat.example.org".to_owned()),
            ..Env::default()
        };
        let s = Settings::load(None, &env, &reader("")).expect("loads");
        assert_eq!(s.password.expose(), "pw-from-env");
    }

    #[test]
    fn load_should_use_the_password_from_the_environment_before_any_file() {
        let env = Env {
            password: Some("pw-from-env".to_owned()),
            ..Env::default()
        };
        let never =
            |_: &Path| -> Result<String, String> { panic!("the password file must not be read") };
        let s = Settings::load(Some(MINIMAL), &env, &never).expect("loads");
        assert_eq!(s.password.expose(), "pw-from-env");
    }

    #[test]
    fn load_should_read_the_password_file_named_in_the_environment_when_the_file_names_another() {
        let env = Env {
            password_file: Some("/run/secrets/env".to_owned()),
            ..Env::default()
        };
        let seen = std::cell::RefCell::new(PathBuf::new());
        let read = |p: &Path| -> Result<String, String> {
            *seen.borrow_mut() = p.to_owned();
            Ok("pw\n".to_owned())
        };
        Settings::load(Some(MINIMAL), &env, &read).expect("loads");
        assert_eq!(*seen.borrow(), PathBuf::from("/run/secrets/env"));
    }

    #[rstest]
    #[case("pw\n", "pw")]
    #[case("pw\r\n", "pw")]
    #[case("pw", "pw")]
    #[case("p w\n\n", "p w")]
    fn load_should_strip_the_line_ending_when_the_password_comes_from_a_file(
        #[case] content: &str,
        #[case] expected: &str,
    ) {
        let s = Settings::load(Some(MINIMAL), &Env::default(), &reader(content)).expect("loads");
        assert_eq!(s.password.expose(), expected);
    }

    #[rstest]
    #[case("")]
    #[case("\n")]
    fn load_should_refuse_an_empty_password_file(#[case] content: &str) {
        assert_eq!(
            Settings::load(Some(MINIMAL), &Env::default(), &reader(content)),
            Err(ConfigError::Missing("password"))
        );
    }

    #[test]
    fn load_should_report_the_password_file_error_without_the_password() {
        let read = |_: &Path| -> Result<String, String> {
            Err("cannot read /run/secrets/xat: entity not found".to_owned())
        };
        let error = Settings::load(Some(MINIMAL), &Env::default(), &read).expect_err("unreadable");
        assert_eq!(
            error,
            ConfigError::PasswordFile("cannot read /run/secrets/xat: entity not found".to_owned())
        );
    }

    #[test]
    fn load_should_name_the_password_when_neither_a_value_nor_a_file_is_set() {
        let file = "jid = \"agent@chat.example.org\"\nallowed = [\"alice@chat.example.org\"]\n";
        assert_eq!(
            load(file, &Env::default()),
            Err(ConfigError::Missing("password"))
        );
    }

    #[test]
    fn load_should_refuse_a_password_written_in_the_file() {
        let file = format!("{MINIMAL}password = \"hunter2-value\"\n");
        let error = load(&file, &Env::default()).expect_err("unknown key");
        assert!(matches!(error, ConfigError::Parse(_)), "{error:?}");
        assert!(!format!("{error} {error:?}").contains("hunter2-value"));
    }

    #[test]
    fn load_should_not_echo_values_when_the_file_has_a_wrong_type() {
        let error = load("jid = 987654321\n", &Env::default()).expect_err("bad type");
        assert!(matches!(error, ConfigError::Parse(_)));
        assert!(!format!("{error} {error:?}").contains("987654321"));
    }

    #[test]
    fn load_should_name_the_line_when_the_file_is_broken() {
        let error = load(
            "jid = \"agent@chat.example.org\"\n\nallowed = 5\n",
            &Env::default(),
        )
        .expect_err("bad type");
        assert!(error.to_string().contains("line 3"), "{error}");
    }

    #[rstest]
    #[case("jid", ConfigError::Missing("jid"))]
    #[case("allowed", ConfigError::Missing("allowed"))]
    fn load_should_name_the_missing_setting_when_a_required_key_is_absent(
        #[case] drop: &str,
        #[case] expected: ConfigError,
    ) {
        let file: String = MINIMAL
            .lines()
            .filter(|l| !l.starts_with(drop))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(load(&file, &Env::default()), Err(expected));
    }

    #[rstest]
    #[case("jid", "agent@chat.example.org/phone")]
    #[case("jid", "agent")]
    #[case("jid", "@chat.example.org")]
    #[case("jid", "agent@")]
    #[case("jid", "a b@chat.example.org")]
    #[case("jid", "")]
    fn load_should_refuse_a_jid_that_is_not_bare(#[case] field: &'static str, #[case] value: &str) {
        let file = MINIMAL.replace("agent@chat.example.org", value);
        assert_eq!(
            load(&file, &Env::default()),
            Err(ConfigError::Invalid {
                field,
                reason: "must be a bare JID: <local>@<domain>".to_owned()
            })
        );
    }

    #[rstest]
    #[case("alice@chat.example.org/phone")]
    #[case("alice")]
    #[case("")]
    #[case("alice@chat.example.org chat.example.org")]
    fn load_should_refuse_an_allowed_entry_that_is_not_bare(#[case] value: &str) {
        let file = MINIMAL.replace("\"alice@chat.example.org\"", &format!("\"{value}\""));
        assert!(matches!(
            load(&file, &Env::default()),
            Err(ConfigError::Invalid {
                field: "allowed",
                ..
            })
        ));
    }

    #[test]
    fn load_should_refuse_an_empty_allowed_list() {
        let file = MINIMAL.replace("[\"alice@chat.example.org\"]", "[]");
        assert_eq!(
            load(&file, &Env::default()),
            Err(ConfigError::Missing("allowed"))
        );
    }

    #[test]
    fn load_should_refuse_an_approver_that_is_not_bare() {
        let file = format!("{MINIMAL}approvers = [\"alice@chat.example.org/phone\"]\n");
        assert!(matches!(
            load(&file, &Env::default()),
            Err(ConfigError::Invalid {
                field: "approvers",
                ..
            })
        ));
    }

    #[test]
    fn load_should_drop_duplicates_and_lowercase_when_jids_repeat() {
        let file = MINIMAL.replace(
            "[\"alice@chat.example.org\"]",
            "[\"alice@chat.example.org\", \"ALICE@chat.example.org\"]",
        );
        assert_eq!(
            load(&file, &Env::default()).expect("loads").allowed,
            ["alice@chat.example.org"]
        );
    }

    #[rstest]
    #[case("1", Some(1))]
    #[case("86400", Some(86_400))]
    #[case("0", None)]
    #[case("86401", None)]
    #[case("-5", None)]
    #[case("soon", None)]
    #[case("", Some(600))]
    fn load_should_accept_an_approval_timeout_between_1_second_and_one_day(
        #[case] value: &str,
        #[case] expected: Option<u64>,
    ) {
        let env = Env {
            approval_timeout_secs: Some(value.to_owned()),
            ..Env::default()
        };
        match (load(MINIMAL, &env), expected) {
            (Ok(s), Some(secs)) => assert_eq!(s.approval_timeout, Duration::from_secs(secs)),
            (
                Err(ConfigError::Invalid {
                    field: "approval_timeout_secs",
                    ..
                }),
                None,
            ) => {}
            (other, expected) => panic!("{value}: {other:?} versus {expected:?}"),
        }
    }

    #[test]
    fn load_should_refuse_a_file_timeout_of_zero() {
        let file = format!("{MINIMAL}approval_timeout_secs = 0\n");
        assert!(matches!(
            load(&file, &Env::default()),
            Err(ConfigError::Invalid {
                field: "approval_timeout_secs",
                ..
            })
        ));
    }

    #[test]
    fn load_should_use_the_default_port_when_a_server_host_has_none() {
        let file = format!("{MINIMAL}server_host = \"xmpp.example.org\"\n");
        assert_eq!(
            load(&file, &Env::default()).expect("loads").server,
            Some(("xmpp.example.org".to_owned(), 5223))
        );
    }

    #[test]
    fn load_should_refuse_a_server_port_without_a_host() {
        let file = format!("{MINIMAL}server_port = 5223\n");
        assert!(matches!(
            load(&file, &Env::default()),
            Err(ConfigError::Invalid {
                field: "server_port",
                ..
            })
        ));
    }

    #[test]
    fn load_should_refuse_server_port_zero() {
        let file = format!("{MINIMAL}server_host = \"xmpp.example.org\"\nserver_port = 0\n");
        assert!(matches!(
            load(&file, &Env::default()),
            Err(ConfigError::Invalid {
                field: "server_port",
                ..
            })
        ));
    }

    #[rstest]
    #[case("agent_command = \"\"")]
    #[case("agent_command = \"   \"")]
    fn load_should_refuse_an_empty_agent_command(#[case] line: &str) {
        let file = format!("{MINIMAL}{line}\n");
        assert!(matches!(
            load(&file, &Env::default()),
            Err(ConfigError::Invalid {
                field: "agent_command",
                ..
            })
        ));
    }

    #[test]
    fn debug_should_hide_the_password_when_the_settings_are_printed() {
        let s = load(MINIMAL, &Env::default()).expect("loads");
        let shown = format!("{s:?} {:?}", s.password);
        assert!(!shown.contains("s3cret-pw"), "{shown}");
        assert!(shown.contains("[redacted]"), "{shown}");
    }

    #[test]
    fn spawn_config_should_run_the_agent_through_env_with_the_secrets_removed() {
        let file = format!(
            "{MINIMAL}agent_command = \"my-agent\"\nagent_args = [\"--acp\"]\ncwd = \"/home/user/work\"\nentrypoint = \"lab\"\n"
        );
        let spawn = load(&file, &Env::default()).expect("loads").spawn_config();
        assert_eq!(spawn.command, "env");
        assert_eq!(
            spawn.args,
            [
                "-u",
                "XAT_PASSWORD",
                "-u",
                "XAT_PASSWORD_FILE",
                "my-agent",
                "--acp"
            ]
        );
        assert_eq!(
            spawn.env,
            [("CLAUDE_CODE_ENTRYPOINT".to_owned(), "lab".to_owned())]
        );
        assert_eq!(spawn.cwd, PathBuf::from("/home/user/work"));
    }

    #[test]
    fn spawn_config_should_not_hand_the_password_to_the_agent_process() {
        let mut s = load(MINIMAL, &Env::default()).expect("loads");
        s.agent_command = "sh".to_owned();
        s.agent_args = vec!["-c".to_owned(), "printf '%s|%s|%s' \"$XAT_PASSWORD\" \"$XAT_PASSWORD_FILE\" \"$CLAUDE_CODE_ENTRYPOINT\"".to_owned()];
        let spawn = s.spawn_config();
        let out = std::process::Command::new(&spawn.command)
            .args(&spawn.args)
            .envs(spawn.env.iter().map(|(k, v)| (k, v)))
            .env("XAT_PASSWORD", "hunter2-value")
            .env("XAT_PASSWORD_FILE", "/run/secrets/xat")
            .output()
            .expect("runs");
        assert_eq!(String::from_utf8_lossy(&out.stdout), "||sandy");
    }

    #[test]
    fn load_should_refuse_an_empty_approvers_list() {
        let file = format!("{MINIMAL}approvers = []\n");
        assert!(matches!(
            load(&file, &Env::default()),
            Err(ConfigError::Invalid {
                field: "approvers",
                ..
            })
        ));
    }

    #[test]
    fn load_should_accept_an_approver_who_may_not_talk_to_the_agent() {
        let file = format!("{MINIMAL}approvers = [\"bob@chat.example.org\"]\n");
        assert_eq!(
            load(&file, &Env::default()).expect("loads").approvers,
            ["bob@chat.example.org"]
        );
    }

    #[rstest]
    #[case("agent_start_timeout_secs = 5", Some(5))]
    #[case("agent_start_timeout_secs = 600", Some(600))]
    #[case("agent_start_timeout_secs = 0", None)]
    #[case("agent_start_timeout_secs = 601", None)]
    #[case("", Some(60))]
    fn load_should_accept_an_agent_start_timeout_between_1_and_600_seconds(
        #[case] line: &str,
        #[case] expected: Option<u64>,
    ) {
        let file = format!("{MINIMAL}{line}\n");
        match (load(&file, &Env::default()), expected) {
            (Ok(s), Some(secs)) => assert_eq!(s.agent_start_timeout, Duration::from_secs(secs)),
            (
                Err(ConfigError::Invalid {
                    field: "agent_start_timeout_secs",
                    ..
                }),
                None,
            ) => {}
            (other, expected) => panic!("{line}: {other:?} versus {expected:?}"),
        }
    }

    #[test]
    fn spawn_config_should_set_the_entrypoint_to_sandy_when_the_file_sets_none() {
        let spawn = load(MINIMAL, &Env::default())
            .expect("loads")
            .spawn_config();
        assert!(
            spawn
                .env
                .contains(&("CLAUDE_CODE_ENTRYPOINT".to_owned(), "sandy".to_owned()))
        );
    }
}
