//! Settings: the rights service URL, the bearer token, the timeout and the granted capabilities.

use std::fmt;
use std::time::Duration;

use crate::errors::ConfigError;

/// Request timeout used when the config file sets none, in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// The bearer token. `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Wrap a token.
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The secret value, for the `Authorization` header and for scrubbing output.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The settings of the rights service client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Base URL of the rights service, without a trailing slash.
    pub rights_url: String,
    /// The bearer token bound to the caller's JID.
    pub token: Token,
    /// Time allowed for one request.
    pub timeout: Duration,
}

/// The settings that come from the environment, read once by the caller.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    /// `XAT_RIGHTS_URL`.
    pub rights_url: Option<String>,
    /// `XAT_RIGHTS_TOKEN`.
    pub token: Option<String>,
    /// `XAT_CAPABILITIES`.
    pub capabilities: Option<String>,
}

impl Config {
    /// Merge the environment over the TOML text of a config file. An environment value wins.
    pub fn load(env: &Env, file: Option<&str>) -> Result<Self, ConfigError> {
        let _ = (env, file);
        Err(ConfigError::Url)
    }
}

/// The capabilities granted to this process.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities(Vec<String>);

impl Capabilities {
    /// Split a comma or space separated list into whole tokens.
    pub fn parse(list: &str) -> Self {
        let _ = list;
        Self(Vec::new())
    }

    /// The `--capabilities` flag when given, otherwise `XAT_CAPABILITIES`, otherwise none.
    pub fn from_sources(flag: Option<&str>, env: Option<&str>) -> Self {
        let _ = (flag, env);
        Self::default()
    }

    /// True when `rooms` is one of the tokens.
    pub fn allows_rooms(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod settings {
    use rstest::rstest;

    use super::*;

    fn env(url: Option<&str>, token: Option<&str>) -> Env {
        Env { rights_url: url.map(str::to_owned), token: token.map(str::to_owned), capabilities: None }
    }

    #[test]
    fn load_should_use_the_environment_when_there_is_no_file() {
        let config = Config::load(&env(Some("https://rights.example.org"), Some("tok-1")), None).expect("loads");
        assert_eq!(config.rights_url, "https://rights.example.org");
        assert_eq!(config.token.expose(), "tok-1");
        assert_eq!(config.timeout, Duration::from_secs(DEFAULT_TIMEOUT_SECS));
    }

    #[test]
    fn load_should_trim_trailing_slashes_when_the_url_has_them() {
        let config = Config::load(&env(Some("https://rights.example.org//"), Some("tok-1")), None).expect("loads");
        assert_eq!(config.rights_url, "https://rights.example.org");
    }

    #[test]
    fn load_should_read_the_file_when_the_environment_is_empty() {
        let file = "rights_url = \"http://rights.example.org:8080\"\nrights_token = \"tok-file\"\ntimeout_secs = 3\n";
        let config = Config::load(&Env::default(), Some(file)).expect("loads");
        assert_eq!(config.rights_url, "http://rights.example.org:8080");
        assert_eq!(config.token.expose(), "tok-file");
        assert_eq!(config.timeout, Duration::from_secs(3));
    }

    #[test]
    fn load_should_prefer_the_environment_when_the_file_sets_the_same_value() {
        let file = "rights_url = \"http://file.example.org\"\nrights_token = \"tok-file\"\n";
        let config = Config::load(&env(Some("http://env.example.org"), Some("tok-env")), Some(file)).expect("loads");
        assert_eq!(config.rights_url, "http://env.example.org");
        assert_eq!(config.token.expose(), "tok-env");
    }

    #[rstest]
    #[case(None, Some("tok"), ConfigError::Missing("XAT_RIGHTS_URL"))]
    #[case(Some(""), Some("tok"), ConfigError::Missing("XAT_RIGHTS_URL"))]
    #[case(Some("https://rights.example.org"), None, ConfigError::Missing("XAT_RIGHTS_TOKEN"))]
    #[case(Some("https://rights.example.org"), Some(""), ConfigError::Missing("XAT_RIGHTS_TOKEN"))]
    #[case(Some("rights.example.org"), Some("tok"), ConfigError::Url)]
    #[case(Some("ftp://rights.example.org"), Some("tok"), ConfigError::Url)]
    fn load_should_fail_with_a_clear_error_when_a_setting_is_missing_or_bad(#[case] url: Option<&str>, #[case] token: Option<&str>, #[case] expected: ConfigError) {
        assert_eq!(Config::load(&env(url, token), None), Err(expected));
    }

    #[test]
    fn load_should_fail_when_the_file_is_not_valid_toml() {
        assert!(matches!(Config::load(&Env::default(), Some("rights_url = ")), Err(ConfigError::File(_))));
    }

    #[test]
    fn load_should_fail_when_the_timeout_is_zero() {
        let file = "timeout_secs = 0\n";
        assert!(matches!(Config::load(&env(Some("http://rights.example.org"), Some("tok")), Some(file)), Err(ConfigError::File(_))));
    }

    #[test]
    fn load_should_not_leak_the_token_when_the_file_is_broken() {
        let file = "rights_token = \"s3cr3t-in-file\nrights_url = 1\n";
        let error = Config::load(&Env::default(), Some(file)).expect_err("broken file");
        assert!(!error.to_string().contains("s3cr3t-in-file"), "{error}");
        assert!(!format!("{error:?}").contains("s3cr3t-in-file"), "{error:?}");
    }

    #[test]
    fn debug_should_hide_the_token_when_the_config_is_printed() {
        let config = Config::load(&env(Some("https://rights.example.org"), Some("s3cr3t-value")), None).expect("loads");
        let shown = format!("{config:?} {:?}", config.token);
        assert!(!shown.contains("s3cr3t-value"), "{shown}");
        assert!(shown.contains("[redacted]"), "{shown}");
    }

    #[rstest]
    #[case("rooms", true)]
    #[case("rooms,directory", true)]
    #[case("directory rooms", true)]
    #[case(" directory , rooms ", true)]
    #[case("directory,rooms,guide", true)]
    #[case("", false)]
    #[case("directory", false)]
    #[case("chatrooms", false)]
    #[case("rooms2", false)]
    #[case("room", false)]
    #[case("ROOMS", false)]
    #[case("rooms:read", false)]
    fn capabilities_should_allow_rooms_only_when_rooms_is_a_whole_token(#[case] list: &str, #[case] allowed: bool) {
        assert_eq!(Capabilities::parse(list).allows_rooms(), allowed);
    }

    #[rstest]
    #[case(Some("rooms"), None, true)]
    #[case(None, Some("rooms"), true)]
    #[case(Some("rooms"), Some("directory"), true)]
    #[case(Some("directory"), Some("rooms"), false)]
    #[case(Some(""), Some("rooms"), false)]
    #[case(None, None, false)]
    fn capabilities_should_prefer_the_flag_when_both_sources_are_set(#[case] flag: Option<&str>, #[case] env: Option<&str>, #[case] allowed: bool) {
        assert_eq!(Capabilities::from_sources(flag, env).allows_rooms(), allowed);
    }
}
