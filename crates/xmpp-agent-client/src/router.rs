//! Classify one chat message: a prompt, a bridge command, a harness command, or a rejection.
//!
//! Port of `src/xmpp_agent_client/router.py`. Claude takes an unknown `/name` as plain text (docs/experiments.md E6), so the bridge
//! validates names against the command list from the init event and stops the rest here. `//name` and `!name` reach the bridge when
//! a client such as Gajim eats a leading slash. A file path such as `/etc/hosts` is a prompt, not a command.
//!
//! Where Python and Rust differ, the Python behaviour is kept: whitespace is Python's (it adds U+001C to U+001F), the name check is
//! an exact `fullmatch`, and suggestions come from a port of `difflib.get_close_matches`.

/// A plain message for the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub text: String,
}

/// A bridge command, with its name lowercased.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Local {
    pub name: String,
    pub args: String,
}

/// A command of the harness, passed through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Harness {
    pub name: String,
    pub args: String,
    /// What to send to the harness: `/name args`.
    pub text: String,
}

/// Why a message was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Empty,
    Unknown,
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    pub reason: Reason,
    pub detail: String,
    pub suggestions: Vec<String>,
}

/// The result of [`route`]; Python returns one of four classes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routed {
    Prompt(Prompt),
    Local(Local),
    Harness(Harness),
    Rejected(Rejected),
}

pub const LOCAL_COMMANDS: [&str; 6] = ["new", "stop", "status", "verbose", "mode", "help"];

/// Python's `str.isspace`: Unicode `White_Space` plus U+001C to U+001F.
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()`.
fn py_strip(s: &str) -> &str {
    s
}

/// `re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9:_.-]*", s)`.
fn is_name(s: &str) -> bool {
    false
}

/// The text after the command prefix, or `None` when the message does not start with one.
fn command_body(text: &str) -> Option<&str> {
    None
}

/// `difflib.SequenceMatcher(None, a, b).ratio()`, with the default `autojunk`.
fn ratio(a: &[char], b: &[char]) -> f64 {
    0.0
}

/// `difflib.get_close_matches(word, possibilities, n=3, cutoff=0.6)`.
fn close_matches(word: &str, possibilities: &[&str]) -> Vec<String> {
    Vec::new()
}

/// Classify one message. `harness_commands` and `terminal_commands` are the command names from the init event.
pub fn route(text: &str, harness_commands: &[&str], terminal_commands: &[&str]) -> Routed {
    Routed::Prompt(Prompt {
        text: text.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // From the claude init event (docs/experiments.md E6): plugin skills carry a colon, terminal-only commands are listed apart.
    const HARNESS: [&str; 10] = [
        "simplify",
        "loop",
        "code-review:code-review",
        "init",
        "security-review",
        "cost",
        "context",
        "compact",
        "clear",
        "status",
    ];
    const TERMINAL: [&str; 4] = ["doctor", "color", "focus", "reload-plugins"];

    fn r(text: &str) -> Routed {
        route(text, &HARNESS, &TERMINAL)
    }

    fn prompt(text: &str) -> Routed {
        Routed::Prompt(Prompt {
            text: text.to_string(),
        })
    }

    fn local(name: &str, args: &str) -> Routed {
        Routed::Local(Local {
            name: name.to_string(),
            args: args.to_string(),
        })
    }

    fn harness(name: &str, args: &str, text: &str) -> Routed {
        Routed::Harness(Harness {
            name: name.to_string(),
            args: args.to_string(),
            text: text.to_string(),
        })
    }

    fn rejected(reason: Reason, detail: &str, suggestions: &[&str]) -> Routed {
        Routed::Rejected(Rejected {
            reason,
            detail: detail.to_string(),
            suggestions: suggestions.iter().map(|s| s.to_string()).collect(),
        })
    }

    fn as_rejected(routed: Routed) -> Rejected {
        match routed {
            Routed::Rejected(rejected) => rejected,
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    // PlainText

    #[test]
    fn plain_text_is_a_prompt() {
        assert_eq!(r("run the tests"), prompt("run the tests"));
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        assert_eq!(r("  run the tests \n"), prompt("run the tests"));
    }

    #[test]
    fn empty_is_rejected() {
        for text in ["", "   ", "\n\t"] {
            assert_eq!(r(text), rejected(Reason::Empty, "", &[]));
        }
    }

    #[test]
    fn slash_inside_text_is_not_a_command() {
        assert_eq!(
            r("see /etc/hosts for details"),
            prompt("see /etc/hosts for details")
        );
    }

    #[test]
    fn file_paths_are_prompts_not_commands() {
        assert_eq!(r("/etc/hosts"), prompt("/etc/hosts"));
        assert_eq!(
            r("/srv/x/notes.md please summarise"),
            prompt("/srv/x/notes.md please summarise")
        );
    }

    #[test]
    fn bang_without_a_name_is_text() {
        for text in ["!!!", "! hello", "!", "!?"] {
            assert_eq!(r(text), prompt(text));
        }
    }

    // LocalCommands

    #[test]
    fn each_local_command() {
        assert_eq!(r("/stop"), local("stop", ""));
        assert_eq!(r("/new"), local("new", ""));
        assert_eq!(r("/status"), local("status", ""));
        assert_eq!(r("/help"), local("help", ""));
        assert_eq!(r("/mode plan"), local("mode", "plan"));
        assert_eq!(r("/verbose on"), local("verbose", "on"));
    }

    #[test]
    fn local_names_are_case_insensitive() {
        assert_eq!(r("/HELP"), local("help", ""));
        assert_eq!(r("/Mode acceptEdits"), local("mode", "acceptEdits")); // args keep their case
    }

    #[test]
    fn local_wins_over_a_harness_command_of_the_same_name() {
        assert!(HARNESS.contains(&"status"));
        assert_eq!(r("/status"), local("status", ""));
    }

    #[test]
    fn args_are_trimmed_but_kept_whole() {
        assert_eq!(r("/mode   plan  "), local("mode", "plan"));
    }

    // Escapes: Gajim has its own / commands and may eat a leading slash; // and ! reach the bridge unchanged.

    #[test]
    fn double_slash_is_a_single_slash() {
        assert_eq!(r("//stop"), local("stop", ""));
        assert_eq!(r("//simplify x"), harness("simplify", "x", "/simplify x"));
    }

    #[test]
    fn bang_is_a_slash() {
        assert_eq!(r("!status"), local("status", ""));
        assert_eq!(
            r("!code-review:code-review 12"),
            harness(
                "code-review:code-review",
                "12",
                "/code-review:code-review 12"
            )
        );
    }

    // HarnessCommands

    #[test]
    fn known_command_is_passed_through_with_its_args() {
        assert_eq!(
            r("/simplify tighten the loop"),
            harness("simplify", "tighten the loop", "/simplify tighten the loop")
        );
    }

    #[test]
    fn plugin_skill_names_with_a_colon() {
        assert_eq!(
            r("/code-review:code-review 12"),
            harness(
                "code-review:code-review",
                "12",
                "/code-review:code-review 12"
            )
        );
    }

    #[test]
    fn command_without_args() {
        assert_eq!(r("/cost"), harness("cost", "", "/cost"));
    }

    #[test]
    fn multiline_args_are_preserved() {
        assert_eq!(
            r("/loop 5m\n/check the build"),
            harness("loop", "5m\n/check the build", "/loop 5m\n/check the build")
        );
    }

    #[test]
    fn harness_names_are_case_sensitive() {
        assert!(matches!(r("/Simplify x"), Routed::Rejected(_)));
    }

    // Rejections

    #[test]
    fn unknown_command_is_rejected_not_sent_to_the_model() {
        // claude would take "/nonexistent foo" as plain text (experiments E6), so the bridge must stop it here.
        let got = as_rejected(r("/nonexistent foo"));
        assert_eq!(got.reason, Reason::Unknown);
        assert_eq!(got.detail, "nonexistent");
    }

    #[test]
    fn close_names_are_suggested() {
        let got = as_rejected(r("/simplfy x"));
        assert_eq!(got.reason, Reason::Unknown);
        assert!(got.suggestions.contains(&"simplify".to_string()));
    }

    #[test]
    fn terminal_only_commands_are_rejected_with_a_reason() {
        for name in TERMINAL {
            assert_eq!(
                r(&format!("/{name}")),
                rejected(Reason::Interactive, name, &[])
            );
        }
    }

    #[test]
    fn lone_slash_is_unknown() {
        assert_eq!(as_rejected(r("/")).reason, Reason::Unknown);
        assert_eq!(as_rejected(r("//")).reason, Reason::Unknown);
    }

    #[test]
    fn no_harness_commands_known_yet_means_every_non_local_command_is_unknown() {
        assert_eq!(
            as_rejected(route("/simplify x", &[], &[])).reason,
            Reason::Unknown
        );
    }

    // Exactness against Python 3.13. Expected values come from running the Python code.

    #[test]
    fn strip_uses_the_python_whitespace_set() {
        assert_eq!(py_strip("\u{1c}\u{1d}\u{1e}\u{1f} x \u{1f}"), "x");
        assert_eq!(py_strip("\u{85}\u{a0}\u{3000}x\u{2028}"), "x");
        assert_eq!(py_strip("\u{200b}x\u{200b}"), "\u{200b}x\u{200b}"); // zero width space is not whitespace
        assert_eq!(py_strip(" \t\n\r\u{b}\u{c}"), "");
    }

    #[test]
    fn route_trims_and_splits_on_the_python_whitespace_set() {
        assert_eq!(r("\u{1c}/stop\u{1d}"), local("stop", ""));
        assert_eq!(r("/mode\u{1c}arg"), local("mode", "arg"));
        assert_eq!(r("/mode\u{1c}"), local("mode", ""));
        assert_eq!(r("/mode\u{85}plan"), local("mode", "plan"));
        assert_eq!(r("\u{a0}/stop"), local("stop", ""));
        assert_eq!(r("\u{1f}"), rejected(Reason::Empty, "", &[]));
    }

    #[test]
    fn leading_space_after_the_slash_shifts_the_args_like_python() {
        // Python slices the unstripped body by the token length, so each leading space eats one char of the args.
        assert_eq!(r("/ mode plan"), local("mode", "e plan"));
        assert_eq!(r("/\u{3000}mode plan"), local("mode", "e plan"));
        assert_eq!(r("/\u{1c}mode x"), local("mode", "e x"));
        assert_eq!(
            r("/ simplify x"),
            harness("simplify", "y x", "/simplify y x")
        );
    }

    #[test]
    fn names_are_ascii_only_and_start_alphanumeric() {
        assert_eq!(r("/-x"), prompt("/-x"));
        assert_eq!(r("/x\u{e9}"), prompt("/x\u{e9}"));
        assert_eq!(r("/stop\u{200b}"), prompt("/stop\u{200b}"));
        assert_eq!(r("/a_b"), rejected(Reason::Unknown, "a_b", &[]));
        assert_eq!(r("!1a"), rejected(Reason::Unknown, "1a", &[]));
        assert_eq!(r("!\u{e9}"), prompt("!\u{e9}"));
    }

    #[test]
    fn name_check_is_a_full_match() {
        for ok in [
            "a",
            "A",
            "0",
            "a:b",
            "a_b",
            "a.b",
            "a-b",
            "code-review:code-review",
            "9-",
        ] {
            assert!(is_name(ok), "{ok}");
        }
        for bad in [
            "",
            ":a",
            "_a",
            ".a",
            "-a",
            "a b",
            "a\n",
            "a/b",
            "a\u{e9}",
            "\u{e9}",
            "a\u{ff10}",
        ] {
            assert!(!is_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn command_body_strips_one_prefix() {
        assert_eq!(command_body("//stop"), Some("stop"));
        assert_eq!(command_body("/stop"), Some("stop"));
        assert_eq!(command_body("///"), Some("/"));
        assert_eq!(command_body("/"), Some(""));
        assert_eq!(command_body("!stop"), Some("stop"));
        assert_eq!(command_body("!9"), Some("9"));
        assert_eq!(command_body("!"), None);
        assert_eq!(command_body("!-"), None);
        assert_eq!(command_body("!!stop"), None);
        assert_eq!(command_body("stop"), None);
        assert_eq!(command_body(" /stop"), None);
    }

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn ratio_matches_python_sequence_matcher() {
        let cases: [(&str, &str, f64); 9] = [
            ("abcd", "bcde", 0.75),
            ("private", "pirate", 0.769_230_769_230_769_3),
            ("aXbXc", "abc", 0.75),
            ("ab", "", 0.0),
            ("qabxcd", "abycdf", 0.666_666_666_666_666_6),
            ("", "x", 0.0),
            ("", "", 1.0),
            ("日本語x", "日本語", 0.857_142_857_142_857_1),
            ("abc", "abc", 1.0),
        ];
        for (a, b, want) in cases {
            assert_eq!(ratio(&chars(a), &chars(b)), want, "{a:?} {b:?}");
        }
    }

    #[test]
    fn ratio_recurses_left_and_right_of_the_longest_block() {
        // Longest block "cde" (3), then "a" on its left: 2*4/10.
        assert_eq!(ratio(&chars("abcde"), &chars("acdeb")), 0.8);
        assert_eq!(
            ratio(&chars("xabcdy"), &chars("zabcdw")),
            0.6666666666666666
        );
        assert_eq!(ratio(&chars("abXcd"), &chars("abYcd")), 0.8);
    }

    #[test]
    fn ratio_long_second_sequence_uses_autojunk_like_python() {
        let ab100 = "ab".repeat(100); // 200 chars: "a" and "b" are popular and leave the index
        assert_eq!(ratio(&chars(&ab100), &chars(&ab100)), 1.0);
        let a199b = format!("{}b", "a".repeat(199));
        let a100b = format!("{}b", "a".repeat(100));
        assert_eq!(
            ratio(&chars(&a100b), &chars(&a199b)),
            0.671_096_345_514_950_1
        );
    }

    fn close(word: &str, possibilities: &[&str]) -> Vec<String> {
        close_matches(word, possibilities)
    }

    #[test]
    fn close_matches_real_command_list() {
        let mut known: Vec<&str> = HARNESS
            .iter()
            .chain(LOCAL_COMMANDS.iter())
            .copied()
            .collect();
        known.sort_unstable();
        known.dedup();
        assert_eq!(close("simplfy", &known), ["simplify"]);
    }

    #[test]
    fn close_matches_orders_ties_by_string_descending() {
        assert_eq!(
            close("abcd", &["abcx", "abcy", "abcz", "abcw"]),
            ["abcz", "abcy", "abcx"]
        );
        assert_eq!(
            close("stat", &["status", "stats", "state", "stay"]),
            ["stats", "state", "status"]
        );
        assert_eq!(
            close("ab", &["ab", "abc", "abd", "ba"]),
            ["ab", "abd", "abc"]
        );
    }

    #[test]
    fn close_matches_orders_by_score_before_string() {
        assert_eq!(close("aaa", &["aaaa", "aa", "aaa"]), ["aaa", "aaaa", "aa"]);
        assert_eq!(close("cxde", &["abcde", "cde"]), ["cde", "abcde"]);
    }

    #[test]
    fn close_matches_returns_at_most_three() {
        assert_eq!(
            close("abcd", &["abcx", "abcy", "abcz", "abcw", "abcv"]).len(),
            3
        );
    }

    #[test]
    fn close_matches_applies_the_cutoff() {
        assert_eq!(close("xyz", &["abc", "def"]), Vec::<String>::new());
        assert_eq!(close("abcd", &["bcde"]), ["bcde"]); // ratio 0.75
        assert_eq!(close("abc", &["abxyz"]), Vec::<String>::new()); // ratio 0.5
        assert_eq!(close("abcde", &["abfgh", "abcgh"]), ["abcgh"]); // 0.4 and 0.6: the cutoff is inclusive
    }

    #[test]
    fn close_matches_handles_unicode_by_char() {
        assert_eq!(
            close("héllo", &["hello", "héllo", "hállo", "héllö"]),
            ["héllo", "héllö", "hállo"]
        );
        assert_eq!(
            close("日本語", &["日本", "日本語x", "本日語"]),
            ["日本語x", "日本", "本日語"]
        );
    }

    #[test]
    fn close_matches_long_word_like_python() {
        let ab100 = "ab".repeat(100);
        let ab50 = "ab".repeat(50);
        let ba60 = "ba".repeat(60);
        assert_eq!(
            close(&ab100, &[&ab50, &ab100, &ba60, "abab"]),
            [ab100.as_str(), ab50.as_str()]
        );
        let a199b = format!("{}b", "a".repeat(199));
        let a100b = format!("{}b", "a".repeat(100));
        assert_eq!(close(&a199b, &[&a100b, &"a".repeat(50)]), [a100b.as_str()]);
    }
}
