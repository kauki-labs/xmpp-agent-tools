//! Classify one chat message: a prompt, a bridge command, a harness command, or a rejection.
//!
//! Port of `src/xmpp_agent_client/router.py`. Claude takes an unknown `/name` as plain text (docs/experiments.md E6), so the bridge
//! validates names against the command list from the init event and stops the rest here. `//name` and `!name` reach the bridge when
//! a client such as Gajim eats a leading slash. A file path such as `/etc/hosts` is a prompt, not a command.
//!
//! Where Python and Rust differ, the Python behaviour is kept: whitespace is Python's (it adds U+001C to U+001F), the name check is
//! an exact `fullmatch`, and suggestions come from a port of `difflib.get_close_matches`.

use std::collections::{BTreeSet, HashMap};

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

/// `difflib` turns on `autojunk` for a second sequence this long.
const AUTOJUNK_MIN_LEN: usize = 200;
const CUTOFF: f64 = 0.6;
const MAX_SUGGESTIONS: usize = 3;

pub const LOCAL_COMMANDS: [&str; 6] = ["new", "stop", "status", "verbose", "mode", "help"];

/// Python's `str.isspace`: Unicode `White_Space` plus U+001C to U+001F.
fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()`.
fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// `re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9:_.-]*", s)`. Plugin skills carry a colon, e.g. `code-review:code-review`.
fn is_name(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '-'))
}

/// The text after the command prefix, or `None` when the message does not start with one.
fn command_body(text: &str) -> Option<&str> {
    if let Some(body) = text.strip_prefix("//") {
        return Some(body);
    }
    if let Some(body) = text.strip_prefix('/') {
        return Some(body);
    }
    // Python tests `text[1].isalnum()`. `char::is_alphanumeric` differs for some non-ASCII characters (combining marks), but
    // such a character starts a token that fails the name check, so the result is a prompt either way.
    text.strip_prefix('!')
        .filter(|body| body.chars().next().is_some_and(char::is_alphanumeric))
}

/// `SequenceMatcher(None, a, b)` as `get_close_matches` uses it, with the default `autojunk`.
struct Matcher<'a> {
    a: &'a [char],
    b: &'a [char],
    /// For each character of `b`, its indices in ascending order; popular characters of a long `b` are left out.
    b2j: HashMap<char, Vec<usize>>,
}

impl<'a> Matcher<'a> {
    fn new(a: &'a [char], b: &'a [char]) -> Self {
        let mut b2j: HashMap<char, Vec<usize>> = HashMap::new();
        for (j, &c) in b.iter().enumerate() {
            b2j.entry(c).or_default().push(j);
        }
        if b.len() >= AUTOJUNK_MIN_LEN {
            let ntest = b.len() / 100 + 1;
            b2j.retain(|_, indices| indices.len() <= ntest);
        }
        Matcher { a, b, b2j }
    }

    /// `find_longest_match(alo, ahi, blo, bhi)` as `(i, j, size)`: earliest in `a`, then earliest in `b`, among the longest.
    fn longest_match(
        &self,
        alo: usize,
        ahi: usize,
        blo: usize,
        bhi: usize,
    ) -> (usize, usize, usize) {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for i in alo..ahi {
            let mut newj2len = HashMap::new();
            for &j in self.b2j.get(&self.a[i]).into_iter().flatten() {
                if j < blo {
                    continue;
                }
                if j >= bhi {
                    break;
                }
                let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).unwrap_or(&0) + 1;
                newj2len.insert(j, k);
                if k > bestsize {
                    (besti, bestj, bestsize) = (i + 1 - k, j + 1 - k, k);
                }
            }
            j2len = newj2len;
        }
        // Extend with equal characters, popular ones included (there is no junk set).
        while besti > alo && bestj > blo && self.a[besti - 1] == self.b[bestj - 1] {
            (besti, bestj, bestsize) = (besti - 1, bestj - 1, bestsize + 1);
        }
        while besti + bestsize < ahi
            && bestj + bestsize < bhi
            && self.a[besti + bestsize] == self.b[bestj + bestsize]
        {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    }

    /// The sum of the sizes of the matching blocks.
    fn matches(&self) -> usize {
        let mut total = 0;
        let mut queue = vec![(0, self.a.len(), 0, self.b.len())];
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = self.longest_match(alo, ahi, blo, bhi);
            if k > 0 {
                total += k;
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        total
    }
}

/// `difflib.SequenceMatcher(None, a, b).ratio()`.
fn ratio(a: &[char], b: &[char]) -> f64 {
    let length = a.len() + b.len();
    if length == 0 {
        return 1.0;
    }
    2.0 * Matcher::new(a, b).matches() as f64 / length as f64
}

/// `difflib.get_close_matches(word, possibilities, n=3, cutoff=0.6)`. Python's quick checks only filter, so the ratio decides.
fn close_matches(word: &str, possibilities: &[&str]) -> Vec<String> {
    let word: Vec<char> = word.chars().collect();
    let mut scored: Vec<(f64, &str)> = possibilities
        .iter()
        .map(|&x| (ratio(&x.chars().collect::<Vec<_>>(), &word), x))
        .filter(|&(score, _)| score >= CUTOFF)
        .collect();
    // `heapq.nlargest(n, [(score, x)])`: score descending, then the string descending.
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| b.1.cmp(a.1)));
    scored
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, x)| x.to_string())
        .collect()
}

/// Classify one message. `harness_commands` and `terminal_commands` are the command names from the init event.
pub fn route(text: &str, harness_commands: &[&str], terminal_commands: &[&str]) -> Routed {
    let text = py_strip(text);
    if text.is_empty() {
        return rejected(Reason::Empty, "", Vec::new());
    }
    let Some(body) = command_body(text) else {
        return Routed::Prompt(Prompt {
            text: text.to_string(),
        });
    };
    let token = body
        .split(is_py_space)
        .find(|s| !s.is_empty())
        .unwrap_or("");
    if !token.is_empty() && !is_name(token) {
        return Routed::Prompt(Prompt {
            text: text.to_string(),
        }); // a path or other text that merely starts with a slash
    }
    if token.is_empty() {
        return rejected(Reason::Unknown, "", Vec::new());
    }
    // Python slices the unstripped body by the token's length in characters; the token is ASCII, so that is its byte length,
    // but leading whitespace can be wider. Skipping chars keeps the quirk: "/ mode plan" gives the args "e plan".
    let rest = body
        .char_indices()
        .nth(token.len())
        .map_or("", |(i, _)| &body[i..]);
    let args = py_strip(rest);
    let lowered = token.to_ascii_lowercase();
    if LOCAL_COMMANDS.contains(&lowered.as_str()) {
        return Routed::Local(Local {
            name: lowered,
            args: args.to_string(),
        });
    }
    if terminal_commands.contains(&token) {
        return rejected(Reason::Interactive, token, Vec::new());
    }
    if harness_commands.contains(&token) {
        let text = if args.is_empty() {
            format!("/{token}")
        } else {
            format!("/{token} {args}")
        };
        return Routed::Harness(Harness {
            name: token.to_string(),
            args: args.to_string(),
            text,
        });
    }
    let known: BTreeSet<&str> = harness_commands
        .iter()
        .chain(LOCAL_COMMANDS.iter())
        .copied()
        .collect();
    let known: Vec<&str> = known.into_iter().collect();
    rejected(Reason::Unknown, token, close_matches(token, &known))
}

fn rejected(reason: Reason, detail: &str, suggestions: Vec<String>) -> Routed {
    Routed::Rejected(Rejected {
        reason,
        detail: detail.to_string(),
        suggestions,
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

    #[test]
    fn ratio_matches_python_on_random_short_pairs() {
        let cases: [(&str, &str, f64); 40] = [
            ("cbbccaac", "ccaabbaa", 0.5),
            ("ccacbbccc", "cac", 0.5),
            ("aa", "acab", 0.6666666666666666),
            ("bcacac", "bacab", 0.7272727272727273),
            ("bcacb", "acbaac", 0.5454545454545454),
            ("ba", "baaca", 0.5714285714285714),
            ("aabb", "baccacb", 0.5454545454545454),
            ("abbaba", "aca", 0.4444444444444444),
            ("a", "baccabca", 0.2222222222222222),
            ("bcb", "bb", 0.8),
            ("abcb", "a", 0.4),
            ("bcc", "aa", 0.0),
            ("bbac", "bbaaaa", 0.6),
            ("acbb", "acbcabac", 0.6666666666666666),
            ("bac", "caac", 0.5714285714285714),
            ("acbbcabaa", "ac", 0.36363636363636365),
            ("accbb", "cbbcaca", 0.5),
            ("abc", "ba", 0.4),
            ("caab", "baaaabb", 0.5454545454545454),
            ("caababbcc", "cacaabbcb", 0.6666666666666666),
            ("a", "ba", 0.6666666666666666),
            ("baaab", "bcacccabb", 0.5714285714285714),
            ("cb", "ba", 0.5),
            ("caccbbca", "aa", 0.4),
            ("bb", "bcccbb", 0.5),
            ("caccabca", "aacabccb", 0.625),
            ("abbac", "cabc", 0.6666666666666666),
            ("babbcbac", "aabacba", 0.6666666666666666),
            ("abb", "c", 0.0),
            ("ba", "bb", 0.5),
            ("abbcc", "cbccbbaa", 0.46153846153846156),
            ("caccb", "bccaca", 0.5454545454545454),
            ("baabaaa", "ba", 0.4444444444444444),
            ("ccbacaabb", "cab", 0.5),
            ("caaabbccc", "cabcaabcb", 0.5555555555555556),
            ("bba", "b", 0.5),
            ("cccbbbcaa", "aa", 0.36363636363636365),
            ("abcbbca", "caabbc", 0.6153846153846154),
            ("bbaaaba", "cbc", 0.2),
            ("bac", "ccbbccaab", 0.3333333333333333),
        ];
        for (a, b, want) in cases {
            assert_eq!(ratio(&chars(a), &chars(b)), want, "{a:?} {b:?}");
        }
    }

    #[test]
    fn ratio_matches_python_on_long_pairs_with_popular_characters() {
        // b has 200 or more chars, so its frequent characters leave the index and only the extension steps can match them.
        let cases: [(&str, &str, f64); 12] = [
            (
                "azxxababaabbazbbbaababzbzaabbaaababbbbzaaaaaabbbbaaxaabbbabbbaaabyaa",
                "abbzazbababaabbbbbaaxaabbbabbbaaabyaaaabbbaabybababaabxayabbbaabbzaababbaabbbaababzbzaabbaaababbbbzaaaaaabbbbbbaabbababaazxxababaabbazbbbaaaaaababaxbbbaababababbabbbaazbbababbbaabbabaabbaaabbabbababbbbaabbbzbabxbbbbx",
                0.16901408450704225,
            ),
            (
                "abbaabbbaabaaaabbyabababyababaabbaaxbaaaabbaabayaaabxabbbbxaabby",
                "aaababbbbbaaayzaabybababaaaaabbaabbaybbzbabaabaaaxbzbaaabbbaaabbaaaabxabbbbxaabbbaabbaazzaaaabbbaaabbaxbbaabababaaaabaabbbbbbabbaabbbabbyabxzzbabaabaaabbaabbbaabaaaabbyababaabbaaxbaaaabbaabaaxbbabbbaabbbaabaaaabbaaaaab",
                0.0070921985815602835,
            ),
            (
                "bbaabbbaaaabbabbabbbbbaabbaxabzaaabbbbabaabababaybyzzaabbbababbayyy",
                "bbaaabababazzbaayabaabxababbbaabbbababbaabbbaaaabbabbabbbbbaabbaxbbbbbbybbaabxbbaxaybbbabbbabbaababbyabbaababbababaaxbbbbaazaxbbbbabbbbaabababayabbaaaaababbbaaabbbbabaabababaybbabbbaazaabbabbbaybaabbbbbbbaaaabbbbaaabaaabba",
                0.02768166089965398,
            ),
            (
                "bbbbbaabbbaaababxabbaxyaaxbbbbaabbabaxbbbaaababxabbaxyaaxzaaaay",
                "aaababbbbbyabaaababbaaabbaaaaaaxabbxababbbaaabbbbazbbabbaababxbbbbbbbaabbbaaababxabbaxyaaxzaaaaabbaabxbaaaaababaabaaaaaabababbbbaabbabbaabbaabzbbabyzbabbabbbbaababayabaaabaaabaababaxbababbbbabbbbbabbbbbyaabyaaabbabaaababbbbbbb",
                0.0,
            ),
            (
                "baabaabaabbabaaxabxaxbbabbbbaaabbbbazbaabaabbyz",
                "aabbyabazbaxaaabaabaabaabbabaaxabxaxbaaaaxxbbabbbayabbbbaabaaaaaabayyaabaaaaaabyabxabaxxzababbxabzazbbbabxbbaaxbaaaabbbabbbabbbbaaabbbbbbayaabbaayabzbababaazbzabbbzababbbaaabbaaaybxbbaaabbbbbaabbbbzaaabbab",
                0.0,
            ),
            (
                "baaabbaabaaaazbyabbabbaaabazxbyabbabbaaabaabbaabxyyaxaaaabbaaa",
                "baababbbaabyzbbaabbbababbbabbbabbbbbbababbbbababbbbyabbbabaababaaaaabaababbabbbabbbazbbbaaaaaabbaaabbabbabaaaababbabaabbbbaaabbbaaaabaaaazbzaaabaxaaaabbaaabbaabaaaazbyabbabbaaabaabbaaaaababaaababbababybbxbxxbaabaab",
                0.021739130434782608,
            ),
            (
                "bbazxabaayxabbaayabbbababaaazzxbbbazabbaxz",
                "bbbababbaabaaabybbazxabaabxabbbxaaaaabybabbbbbaabazbbbzbbabababbaabaaabaaaaayaabbaazaaabbbbaaabbbazabbybbbababayaababyybbaaabbbbyaaaaabbybbabbaayabbbababaaabababbbbxbaxybbbbbbzabzbaaababaaaababbbbxzababxbbabyababb",
                0.01568627450980392,
            ),
            (
                "babbbabbbbzababaaybxbxxabazbzzbbazybabbbyababbbbbabaaaabbbbaaabbaa",
                "aabababaabbbabzaaaabbbbabbyaababxaabbaazabbbaaxaabbzabzbaaazbaaabbababbababxbaaaabbaabbaababbaabbxyaababaababbbbbabaaaabbbbaaabbabyaaaabbbazybabbbaxzbabbaaabbaabaaaabxaababbbabbbabbbbzababaaybxbxxabaababbabaaababbba",
                0.0,
            ),
            (
                "abaaaaaabbzbzxaaaabbbaabxyaxbxxayabaaabababz",
                "babbaaaabaabababaaababazbaabaaaaaabbaaabbbaabbaabaybbabaaababaababbaaybbaabybbabbbaaaaxbaybybababbaxbbbbbbabbbbbbbbaaaabxxayabaaabababaabababaaaabbbaabaaaabbbabbbbbaabababbbabbybaaayababaaaaaabbbaaaaabaabbabbbbbabaababaxabaazbb",
                0.02214022140221402,
            ),
            (
                "yaabbabaazbbzzzbabbbbbabbabaababaabayxbazbbbabazby",
                "bbabbbbbabbabaababaabaabbybbbaybaayabbbaabaaabbabaabaaaabyzbbbaabbbbbabyaabbabaazbbbabazbbbbabaaaabaabbxzbyabaazabababaxbbaaxababbabbbabyzaazaabbabaabbabbbazbbbaxbbbxbabaaaayaaazbabaabaaababbbabbbbaababbabbzabaaaaabababa",
                0.0,
            ),
            (
                "bbabbaabbbxbabyxxaaabaabaaaaabzaazabaxaabbaxbbababaayabaabaaaay",
                "bbbybbaababaabbzbbabbaabbbxbabybaxaaaaabbxxbbababaayabaabaaaaabzaazabaxaabbaaaayabbbbababbabbaababaaaaabaaxaaaabbaababbazaabazbaabaabbbbbabbbabaabbbbaxxaabbabbbbbbabaaaxaababybbaabbabaabaaabbbaxbbazbb",
                0.015209125475285171,
            ),
            (
                "baaabbabbbbyaxaaabbbaaabbaabbaaaaaabbxbbbbbzbbbbabababbaaba",
                "axbbabbzzabbababzbaabbaaazxaabbbababbaaaabaaaxbbzabazabayaaabyaxaaabbbaaabbaabbabbbbbaaybzaaaaabbxbbbbbzbbbbabababbaababybababbxbbbabbbybababababaaaabababbbaaabbabbbbabaababybbbabbababxbxaabaxbbbbabbbabaabyaabbbybbazbbaa",
                0.0,
            ),
        ];
        for (a, b, want) in cases {
            assert_eq!(ratio(&chars(a), &chars(b)), want);
        }
    }

    #[test]
    fn ratio_matches_the_block_to_the_right_of_a_long_block() {
        // The longest block "abc" sits at a[3..6] with another match "rst" after it; Python gives 0.8.
        assert_eq!(ratio(&chars("qqqabcrst"), &chars("abcrst")), 0.8);
    }

    #[test]
    fn ratio_extends_a_rare_match_through_popular_characters_both_ways() {
        // "x" is the only indexed character of b; the 30 "a" on each side are found by the extension steps. Python gives 0.4341...
        let a = format!("{}x{}", "a".repeat(30), "a".repeat(30));
        let b = format!("{}x{}", "a".repeat(100), "a".repeat(119));
        assert_eq!(ratio(&chars(&a), &chars(&b)), 0.434_163_701_067_615_65);
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
