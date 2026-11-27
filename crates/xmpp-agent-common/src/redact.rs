//! Redact secrets and describe a tool call for chat.
//!
//! Port of `src/xmpp_agent_common/redact.py`. Tool arguments can hold tokens, and an approval request lands in the room archive.
//! [`describe_tool`] redacts first and cuts second, so a token can never survive as a prefix. File content and edit text are
//! never shown, only their size. The patterns cover common token shapes and key=value pairs; an unknown secret format will
//! pass, which is why arguments are also cut short.

use std::io;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::ser::{Formatter, Serializer};
use serde_json::{Map, Value};

/// Default length limit of [`describe_tool`], in characters.
pub const DEFAULT_MAX_CHARS: usize = 400;

/// Length limit of one argument of an unknown tool, in characters.
const ARGUMENT_MAX_CHARS: usize = 80;

/// Python's `re.IGNORECASE` also folds U+0131 and U+0130 (dotless and dotted capital i) to `i`; the `regex` crate does not, so each `i`
/// of a keyword and each letter class that can hold an `i` names them. Turkish lowercasing of `API_KEY` gives `apı_key`.
const KEYWORDS: &str = "(?:password|passwd|pwd|secret|token|ap[iıİ][_-]?key|credent[iıİ]al)";

/// What Python's `\s` matches in a `str` pattern that the `regex` crate's `\s` does not: U+001C to U+001F. The patterns below write
/// `<WS>` for the whole set, so `[<WS>]` is `\s` and `[^<WS>]` is `\S`.
const WHITESPACE: &str = r"\s\x1c-\x1f";

/// Ordered (pattern, replacement) pairs; the replacement uses the `regex` crate syntax (`${1}`).
static PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    let flag_value = format!(
        r#"(?i)(--?[A-Za-z0-9_ıİ-]*{KEYWORDS}[A-Za-z0-9_ıİ-]*)([<WS>]+)("[^"]*"|'[^']*'|[^<WS>]+)"#
    );
    let name_value = format!(
        r#"(?i)\b([A-Za-z0-9_.ıİ-]*{KEYWORDS}[A-Za-z0-9_.ıİ-]*)(["']?[<WS>]*[=:][<WS>]*)("[^"]*"|'[^']*'|[^<WS>]+)"#
    );
    let patterns: [(&str, &'static str); 11] = [
        (
            r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
            "[redacted private key]",
        ),
        // The Python pattern has a lookbehind for `://`; the `regex` crate has none, so the scheme is captured and put back.
        (r"(://)[^/<WS>:@]+:[^/<WS>@]+@", "${1}[redacted]@"),
        (
            r#"(?i)(author[iıİ]zat[iıİ]on[<WS>]*:[<WS>]*)(?:(?:bearer|basic|token)[<WS>]+)?[^<WS>"']+"#,
            "${1}[redacted]",
        ),
        (
            r"(?i)\bbearer[<WS>]+[A-Za-z0-9._~+/=ıİ-]{10,}",
            "Bearer [redacted]",
        ),
        (r"\bAKIA[0-9A-Z]{16}\b", "[redacted aws key]"),
        (
            r"\bgh[pousr]_[A-Za-z0-9]{20,}\b|\bgithub_pat_[A-Za-z0-9_]{20,}\b",
            "[redacted token]",
        ),
        (r"\bxox[abprs]-[A-Za-z0-9-]{10,}", "[redacted token]"),
        (r"\bsk-[A-Za-z0-9_-]{16,}", "[redacted token]"),
        (
            r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
            "[redacted token]",
        ),
        // --password 'x' (a space separates a flag from its value)
        (&flag_value, "${1}${2}[redacted]"),
        // NAME=value, name: value, "name": "value"
        (&name_value, "${1}${2}[redacted]"),
    ];
    patterns
        .into_iter()
        .map(|(pattern, replacement)| {
            let pattern = pattern.replace("<WS>", WHITESPACE);
            (
                Regex::new(&pattern).expect("a constant pattern is valid"),
                replacement,
            )
        })
        .collect()
});

/// Replace secrets in `text` with placeholders. Applying it twice changes nothing more.
pub fn redact(text: &str) -> String {
    PATTERNS
        .iter()
        .fold(text.to_owned(), |text, (pattern, replacement)| {
            pattern.replace_all(&text, *replacement).into_owned()
        })
}

/// Cut `text` to `max_chars` characters, ending in a marker that gives the cut size. A limit shorter than the marker keeps only the marker.
fn cut(text: &str, max_chars: usize) -> String {
    let length = text.chars().count();
    if length <= max_chars {
        return text.to_owned();
    }
    // The digit count of `length` is an upper bound for the exact one below.
    let marker_length = format!(" … [+{length} more chars]").chars().count();
    let keep = max_chars.saturating_sub(marker_length);
    let kept: String = text.chars().take(keep).collect();
    format!("{kept} … [+{} more chars]", length - keep)
}

/// Writes JSON the way Python's `json.dumps` does: `", "` and `": "` as separators, non-ASCII text as is.
struct PythonSeparators;

impl Formatter for PythonSeparators {
    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if first {
            Ok(())
        } else {
            writer.write_all(b", ")
        }
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }
}

/// A string as is, anything else as JSON text.
fn text_of(value: &Value) -> String {
    if let Value::String(text) = value {
        return text.clone();
    }
    let mut buffer = Vec::new();
    match value.serialize(&mut Serializer::with_formatter(
        &mut buffer,
        PythonSeparators,
    )) {
        Ok(()) => String::from_utf8_lossy(&buffer).into_owned(),
        Err(_) => value.to_string(),
    }
}

/// Python truthiness: absent, null, false, zero and empty values are false.
fn is_set(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(items)) => !items.is_empty(),
        Some(Value::Bool(true)) => true,
    }
}

/// Python's `str.rstrip()`, which also strips the information separators U+001C to U+001F.
fn rstrip(text: &str) -> &str {
    text.trim_end_matches(|c: char| c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c))
}

/// Describe a tool call in at most `max_chars` characters, redacted first and cut second.
///
/// The result is meant for chat. It never contains file content or edit text, only their size, and a `tool_input` that is not a
/// JSON object counts as empty.
pub fn describe_tool(tool_name: &str, tool_input: &Value, max_chars: usize) -> String {
    let empty = Map::new();
    let input = tool_input.as_object().unwrap_or(&empty);
    let field = |key: &str| input.get(key).map_or_else(String::new, text_of);
    let set = |key: &str| is_set(input.get(key));
    let size = |key: &str| field(key).chars().count();
    let with_path = |label: &str| {
        if set("path") {
            format!("{label} {} in {}", field("pattern"), field("path"))
        } else {
            format!("{label} {}", field("pattern"))
        }
    };

    let out = match tool_name {
        "Bash" if set("description") => {
            format!("Bash: {} ({})", field("command"), field("description"))
        }
        "Bash" => format!("Bash: {}", field("command")),
        "Write" => format!("Write {} ({} chars)", field("file_path"), size("content")),
        "Edit" => format!(
            "Edit {} (replace {} chars with {} chars)",
            field("file_path"),
            size("old_string"),
            size("new_string")
        ),
        "MultiEdit" => {
            let edits = input
                .get("edits")
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            format!("MultiEdit {} ({edits} edits)", field("file_path"))
        }
        "NotebookEdit" => format!("NotebookEdit {}", field("notebook_path")),
        "Read" => format!("Read {}", field("file_path")),
        "Glob" => with_path("Glob"),
        "Grep" => with_path("Grep"),
        "WebFetch" => format!("WebFetch {}", field("url")),
        "WebSearch" => format!("WebSearch {}", field("query")),
        _ => {
            let pairs: Vec<String> = input
                .iter()
                .map(|(key, value)| {
                    format!(
                        "{key}={}",
                        cut(&redact(&text_of(value)), ARGUMENT_MAX_CHARS)
                    )
                })
                .collect();
            rstrip(&format!("{tool_name} {}", pairs.join(", "))).to_owned()
        }
    };
    cut(&redact(&out), max_chars)
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

    #[test]
    fn redact_should_treat_information_separators_as_whitespace_when_matching_like_python() {
        // Python's `\s` matches U+001C to U+001F, the `regex` crate's does not.
        assert_eq!(redact("token: \u{1f}"), "token: \u{1f}");
        assert_eq!(redact("PWD=1\u{1f}x"), "PWD=[redacted]\u{1f}x");
    }

    #[test]
    fn redact_should_match_keywords_when_a_dotless_i_replaces_i() {
        // Python's `re.IGNORECASE` folds U+0131 to `i`; the `regex` crate does not. Turkish lowercasing of `API_KEY` gives `apı_key`.
        assert_eq!(redact("apı_key=abc123"), "apı_key=[redacted]");
        assert_eq!(redact("credentıal=hunter2"), "credentıal=[redacted]");
        assert_eq!(redact("authorızation: abc123"), "authorızation: [redacted]");
        assert_eq!(redact("apİ_key=abc"), "apİ_key=[redacted]");
        assert_eq!(redact("xıtoken=abc"), "xıtoken=[redacted]");
        assert_eq!(redact("--ıpassword x"), "--ıpassword [redacted]");
        assert_eq!(redact("--passwordı x"), "--passwordı [redacted]");
        assert_eq!(redact("bearer abcdefghijkİſ@"), "Bearer [redacted]@");
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

    // Beyond the Python tests: exact output per tool, written to kill the mutants `cargo mutants` found in the 1:1 port.

    #[test]
    fn describe_tool_should_format_each_known_tool_when_the_input_is_complete() {
        let cases = [
            (
                "Bash",
                json!({"command": "ls", "description": "List"}),
                "Bash: ls (List)",
            ),
            ("Bash", json!({"command": "ls"}), "Bash: ls"),
            (
                "Write",
                json!({"file_path": "/a", "content": "héllo"}),
                "Write /a (5 chars)",
            ),
            (
                "Edit",
                json!({"file_path": "/a", "old_string": "ab", "new_string": "c"}),
                "Edit /a (replace 2 chars with 1 chars)",
            ),
            (
                "MultiEdit",
                json!({"file_path": "/a", "edits": [{}, {}]}),
                "MultiEdit /a (2 edits)",
            ),
            (
                "MultiEdit",
                json!({"file_path": "/a", "edits": "no"}),
                "MultiEdit /a (0 edits)",
            ),
            (
                "NotebookEdit",
                json!({"notebook_path": "/n.ipynb"}),
                "NotebookEdit /n.ipynb",
            ),
            ("Read", json!({"file_path": "/a"}), "Read /a"),
            (
                "Glob",
                json!({"pattern": "*.rs", "path": "src"}),
                "Glob *.rs in src",
            ),
            ("Glob", json!({"pattern": "*.rs"}), "Glob *.rs"),
            (
                "Grep",
                json!({"pattern": "x", "path": "src"}),
                "Grep x in src",
            ),
            ("Grep", json!({"pattern": "x"}), "Grep x"),
            (
                "WebFetch",
                json!({"url": "https://example.com"}),
                "WebFetch https://example.com",
            ),
            ("WebSearch", json!({"query": "rust"}), "WebSearch rust"),
        ];
        for (name, input, expected) in cases {
            assert_eq!(describe(name, input), expected);
        }
    }

    #[test]
    fn describe_tool_should_skip_the_description_when_it_is_empty_or_zero_or_null() {
        for empty in [
            json!(""),
            json!(0),
            json!(0.0),
            json!(false),
            json!(null),
            json!([]),
            json!({}),
        ] {
            assert_eq!(
                describe("Bash", json!({"command": "ls", "description": empty})),
                "Bash: ls"
            );
        }
        for set in [
            json!("x"),
            json!(1),
            json!(true),
            json!([1]),
            json!({"a": 1}),
        ] {
            assert!(describe("Bash", json!({"command": "ls", "description": set})).contains(" ("));
        }
    }

    #[test]
    fn describe_tool_should_write_json_like_python_when_an_argument_is_nested() {
        let out = describe("t", json!({"a": [1, "b"], "c": {"d": null, "e": 2.5}}));
        assert_eq!(out, r#"t a=[1, "b"], c={"d": null, "e": 2.5}"#);
    }

    #[test]
    fn describe_tool_should_strip_trailing_whitespace_when_the_tool_is_unknown() {
        assert_eq!(describe("t", json!({})), "t");
        assert_eq!(describe("t", json!({"a": "x\u{1f}"})), "t a=x");
    }
}
