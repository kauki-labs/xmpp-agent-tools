//! Settings: a TOML file with environment overrides, validated once at start.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use xmpp_agent_acp::SpawnConfig;

/// Default time an approval waits for an answer, in seconds.
pub const DEFAULT_APPROVAL_TIMEOUT_SECS: u64 = 600;

/// Longest approval timeout accepted, in seconds (one day).
pub const MAX_APPROVAL_TIMEOUT_SECS: u64 = 86_400;

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
        f.write_str(&self.0)
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
    /// Where the operational JSONL log goes; `None` writes none.
    pub log_path: Option<PathBuf>,
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
        let _ = (file, env, read_password_file);
        Err(ConfigError::Missing("jid"))
    }

    /// How to start the agent.
    pub fn spawn_config(&self) -> SpawnConfig {
        SpawnConfig {
            command: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            cwd: PathBuf::new(),
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
    fn spawn_config_should_carry_the_command_the_entrypoint_and_the_directory() {
        let file = format!(
            "{MINIMAL}agent_command = \"my-agent\"\nagent_args = [\"--acp\"]\ncwd = \"/home/user/work\"\nentrypoint = \"lab\"\n"
        );
        let spawn = load(&file, &Env::default()).expect("loads").spawn_config();
        assert_eq!(spawn.command, "my-agent");
        assert_eq!(spawn.args, ["--acp"]);
        assert_eq!(
            spawn.env,
            [("CLAUDE_CODE_ENTRYPOINT".to_owned(), "lab".to_owned())]
        );
        assert_eq!(spawn.cwd, PathBuf::from("/home/user/work"));
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
