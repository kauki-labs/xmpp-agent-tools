//! Redact secrets and describe a tool call for chat.
//!
//! Port of `src/xmpp_agent_common/redact.py`.

use serde_json::Value;

/// Default length limit of [`describe_tool`], in characters.
pub const DEFAULT_MAX_CHARS: usize = 400;

/// Replace secrets in `text` with placeholders.
pub fn redact(text: &str) -> String {
    text.to_owned()
}

/// Describe a tool call in at most `max_chars` characters, redacted first and cut second.
pub fn describe_tool(_tool_name: &str, _tool_input: &Value, _max_chars: usize) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // Fake secrets built from pieces so scanners do not flag this file.
    const AWS: &str = concat!("AKIA", "ABCDEFGHIJKLMNOP");
    const GH: &str = concat!("ghp_", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
    const SK: &str = concat!("sk-ant-", "api03-AbCdEfGhIjKlMnOpQrStUvWx");
    const SLACK: &str = concat!("xoxb-", "1234567890-abcdefghij");
    const JWT: &str = concat!(
        "eyJhbGciOiJIUzI1NiJ9",
        ".",
        "eyJzdWIiOiIxMjM0NTY3ODkwIn0",
        ".",
        "SflKxwRJSMeKKF2QT4fwpMeJf36"
    );
    const PEM: &str = concat!(
        "-----BEGIN ",
        "RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1234\nabcdefg\n-----END ",
        "RSA PRIVATE KEY-----"
    );

    fn describe(name: &str, input: Value) -> String {
        describe_tool(name, &input, DEFAULT_MAX_CHARS)
    }

    // Redact

    #[test]
    fn redact_should_keep_plain_text_when_there_is_no_secret() {
        for text in [
            "ls -la /etc/hosts",
            "git checkout 3f2a9c1",
            "python3 -m unittest",
            "echo hello world",
            "",
        ] {
            assert_eq!(redact(text), text);
        }
    }

    #[test]
    fn redact_should_hide_the_secret_when_it_has_a_known_token_shape() {
        for secret in [AWS, GH, SK, SLACK, JWT] {
            let out = redact(&format!("curl -H 'x: {secret}' https://example.com"));
            assert!(!out.contains(secret), "{secret} leaked: {out}");
            assert!(out.contains("[redacted"), "no placeholder: {out}");
        }
    }

    #[test]
    fn redact_should_hide_a_private_key_when_the_block_spans_lines() {
        let out = redact(&format!("cat > key.pem <<EOF\n{PEM}\nEOF"));
        assert!(!out.contains("MIIEow"));
        assert!(!out.contains("abcdefg"));
        assert!(out.contains("EOF"));
    }

    #[test]
    fn redact_should_keep_the_name_when_a_key_value_pair_holds_a_secret() {
        assert_eq!(redact("export API_KEY=abc123"), "export API_KEY=[redacted]");
        assert_eq!(
            redact("DB_PASSWORD=hunter2 psql -h db"),
            "DB_PASSWORD=[redacted] psql -h db"
        );
        assert_eq!(redact("token: abcdef"), "token: [redacted]");
        assert_eq!(
            redact(r#"client_secret="s3cr3t value""#),
            "client_secret=[redacted]"
        );
        assert_eq!(redact("--password 'pw'"), "--password [redacted]");
    }

    #[test]
    fn redact_should_hide_the_token_when_it_is_in_an_authorization_header() {
        let out = redact(r#"curl -H "Authorization: Bearer abc.def.ghi-jkl" https://x"#);
        assert!(!out.contains("abc.def.ghi-jkl"));
        assert!(out.contains("Authorization"));
    }

    #[test]
    fn redact_should_hide_the_value_when_an_authorization_header_uses_another_scheme() {
        // not covered by the generic bearer rule
        for (header, secret) in [
            ("Authorization: Basic dXNlcjpwYXNz", "dXNlcjpwYXNz"),
            ("Authorization: abc123", "abc123"),
            ("authorization:token zzz999", "zzz999"),
        ] {
            let out = redact(&format!("curl -H '{header}' https://x"));
            assert!(!out.contains(secret), "{secret} leaked: {out}");
            assert!(out.contains("uthorization"), "{out}");
        }
    }

    #[test]
    fn redact_should_hide_a_bearer_token_when_it_stands_alone() {
        assert!(!redact("Bearer abcdefghij1234").contains("abcdefghij1234"));
    }

    #[test]
    fn redact_should_hide_the_password_when_a_url_holds_credentials() {
        let out = redact("git clone https://user:pw123@github.com/org/repo.git");
        assert!(!out.contains("pw123"));
        assert!(out.contains("github.com/org/repo.git"));
    }

    #[test]
    fn redact_should_hide_every_secret_when_one_line_holds_several() {
        let out = redact(&format!("A_TOKEN=one B_SECRET=two {GH} {AWS}"));
        for leaked in ["one", "two", GH, AWS] {
            assert!(!out.contains(leaked), "{leaked} leaked: {out}");
        }
    }

    #[test]
    fn redact_should_return_the_same_text_when_applied_twice() {
        let once = redact(&format!("export TOKEN=abc {GH}"));
        assert_eq!(redact(&once), once);
    }

    #[test]
    fn redact_should_keep_words_that_merely_contain_a_keyword() {
        // "author" and "tokenize" are not secrets
        assert_eq!(redact("git log --author=bob"), "git log --author=bob");
        assert_eq!(redact("python tokenize.py"), "python tokenize.py");
    }

    // Describe

    #[test]
    fn describe_tool_should_show_the_redacted_command_when_the_tool_is_bash() {
        let out = describe(
            "Bash",
            json!({"command": "DB_PASSWORD=hunter2 psql -h db", "description": "Query the db"}),
        );
        assert!(out.contains("psql -h db"));
        assert!(!out.contains("hunter2"));
        assert!(out.starts_with("Bash"));
    }

    #[test]
    fn describe_tool_should_not_show_content_when_the_tool_is_write() {
        let content = "token=abcdef-secret-content";
        let out = describe(
            "Write",
            json!({"file_path": "/tmp/a.txt", "content": content}),
        );
        assert!(out.contains("/tmp/a.txt"));
        assert!(!out.contains("abcdef-secret-content"));
        assert!(out.contains(&content.chars().count().to_string())); // the size is shown, the text is not
    }

    #[test]
    fn describe_tool_should_not_show_old_or_new_text_when_the_tool_is_edit() {
        let out = describe(
            "Edit",
            json!({"file_path": "/src/x.py", "old_string": "def helper_function_name():", "new_string": "def renamed_thing():"}),
        );
        assert!(out.contains("/src/x.py"));
        for leaked in ["helper_function_name", "renamed_thing"] {
            // plain text, so only the omission hides it
            assert!(!out.contains(leaked), "{leaked} leaked: {out}");
        }
    }

    #[test]
    fn describe_tool_should_redact_the_pattern_when_the_tool_reads_or_searches() {
        assert!(describe("Read", json!({"file_path": "/etc/hosts"})).contains("/etc/hosts"));
        let out = describe("Grep", json!({"pattern": "API_KEY=abc123", "path": "src"}));
        assert!(!out.contains("abc123"));
        assert!(out.contains("src"));
    }

    #[test]
    fn describe_tool_should_redact_url_credentials_when_the_tool_is_web_fetch() {
        let out = describe(
            "WebFetch",
            json!({"url": "https://user:pw123@example.com/a"}),
        );
        assert!(!out.contains("pw123"));
        assert!(out.contains("example.com/a"));
    }

    #[test]
    fn describe_tool_should_show_redacted_arguments_when_the_tool_is_unknown() {
        let out = describe(
            "mcp__stub__thing",
            json!({"name": "x", "api_key": "abc123", "nested": {"token": "zzz999"}}),
        );
        assert!(out.contains("mcp__stub__thing"));
        for leaked in ["abc123", "zzz999"] {
            assert!(!out.contains(leaked), "{leaked} leaked: {out}");
        }
    }

    #[test]
    fn describe_tool_should_cut_to_the_limit_when_a_value_is_long() {
        let out = describe_tool(
            "Bash",
            &json!({"command": format!("echo {}", "x".repeat(5000))}),
            200,
        );
        assert!(out.chars().count() <= 200);
        assert!(out.contains("more")); // says that something was cut
    }

    #[test]
    fn describe_tool_should_hide_a_secret_when_the_cut_would_split_it() {
        // redact first, then cut. Unredacted this is 155 chars, so a cut-first version cuts inside the token (about 14 chars of it
        // survive); redacted it is 131 chars and needs no cut.
        let command = format!("curl {} -H {GH}", "a".repeat(100));
        let out = describe_tool("Bash", &json!({ "command": command }), 150);
        assert!(!out.contains(&GH[..12]), "{out}");
    }

    #[test]
    fn describe_tool_should_hide_a_secret_when_the_per_argument_cut_would_split_it() {
        // 91 chars unredacted: a per-argument cut at 80 keeps about 8 chars of the token (too short for the final pass to match);
        // redacted first it is 67 chars and needs no cut.
        let blob = format!("{} {GH}", "x".repeat(50));
        let out = describe("mcp__x__y", json!({ "blob": blob }));
        assert!(!out.contains(&GH[..6]), "{out}");
    }

    #[test]
    fn describe_tool_should_not_panic_when_fields_are_missing() {
        // The Python test asserts `str`; the Rust return type guarantees that, so this checks that no case panics and the name is kept.
        for name in [
            "Bash", "Write", "Edit", "Read", "Grep", "WebFetch", "Whatever",
        ] {
            assert!(describe(name, json!({})).starts_with(name));
        }
    }
}
