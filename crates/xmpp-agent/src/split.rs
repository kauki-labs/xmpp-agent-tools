//! Cut an agent's answer into chat messages.

/// Longest chat message, in characters. A longer answer is split.
pub const MAX_MESSAGE_CHARS: usize = 3000;

/// Split `text` into messages of at most `max_chars` characters, on line boundaries.
///
/// A line longer than `max_chars` is cut at a character boundary. Blank text gives no message. Joining the messages with a newline gives
/// the text again, except that a cut line loses no character and blank edges are trimmed.
pub fn split_message(text: &str, max_chars: usize) -> Vec<String> {
    let max = max_chars.max(1);
    let mut messages: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    let mut started = false;
    fn flush(
        messages: &mut Vec<String>,
        current: &mut String,
        current_len: &mut usize,
        started: &mut bool,
    ) {
        let message = current.trim_matches('\n');
        if !message.trim().is_empty() {
            messages.push(message.to_owned());
        }
        current.clear();
        *current_len = 0;
        *started = false;
    }
    for line in text.lines() {
        let len = line.chars().count();
        if len > max {
            flush(&mut messages, &mut current, &mut current_len, &mut started);
            let chars: Vec<char> = line.chars().collect();
            let mut pieces = chars
                .chunks(max)
                .map(|piece| piece.iter().collect::<String>())
                .collect::<Vec<_>>();
            let last = pieces.pop().unwrap_or_default();
            for piece in pieces {
                messages.push(piece);
            }
            current_len = last.chars().count();
            current = last;
            started = true;
            continue;
        }
        let joined = if started { current_len + 1 + len } else { len };
        if started && joined > max {
            flush(&mut messages, &mut current, &mut current_len, &mut started);
        }
        if started {
            current.push('\n');
            current_len += 1;
        }
        current.push_str(line);
        current_len += len;
        started = true;
    }
    flush(&mut messages, &mut current, &mut current_len, &mut started);
    messages
}

#[cfg(test)]
mod splitting {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("")]
    #[case("   ")]
    #[case("\n\n")]
    fn split_message_should_return_nothing_when_the_text_is_blank(#[case] text: &str) {
        assert!(split_message(text, 10).is_empty());
    }

    #[rstest]
    #[case("hello", 10, &["hello"])]
    #[case("hello", 5, &["hello"])]
    #[case("a\nb\nc", 3, &["a\nb", "c"])]
    #[case("a\nb\nc", 5, &["a\nb\nc"])]
    #[case("ab\ncd\nef", 5, &["ab\ncd", "ef"])]
    #[case("a\n\nb", 2, &["a", "b"])]
    #[case("\n\nhello\n\n", 10, &["hello"])]
    fn split_message_should_pack_whole_lines_when_the_text_has_lines(
        #[case] text: &str,
        #[case] max: usize,
        #[case] expected: &[&str],
    ) {
        assert_eq!(split_message(text, max), expected);
    }

    #[rstest]
    #[case("xxxxxxx", 3, &["xxx", "xxx", "x"])]
    #[case("ab\nxxxxxxx", 3, &["ab", "xxx", "xxx", "x"])]
    #[case("xxxxxxx\nab", 3, &["xxx", "xxx", "x", "ab"])]
    #[case("xxxxxxxx\nab", 3, &["xxx", "xxx", "xx", "ab"])]
    fn split_message_should_cut_a_long_line_when_it_exceeds_the_limit(
        #[case] text: &str,
        #[case] max: usize,
        #[case] expected: &[&str],
    ) {
        assert_eq!(split_message(text, max), expected);
    }

    #[test]
    fn split_message_should_count_characters_not_bytes_when_the_text_is_not_ascii() {
        let text = "\u{e9}".repeat(7);
        assert_eq!(
            split_message(&text, 3),
            ["\u{e9}\u{e9}\u{e9}", "\u{e9}\u{e9}\u{e9}", "\u{e9}"]
        );
        let emoji = "\u{1f600}".repeat(5);
        assert_eq!(split_message(&emoji, 2).len(), 3);
    }

    #[test]
    fn split_message_should_keep_every_message_within_the_limit_and_lose_nothing() {
        let text = (0..400)
            .map(|n| format!("line number {n} with some words"))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = split_message(&text, MAX_MESSAGE_CHARS);
        assert!(messages.len() > 1);
        assert!(
            messages
                .iter()
                .all(|m| m.chars().count() <= MAX_MESSAGE_CHARS)
        );
        assert_eq!(messages.join("\n"), text);
    }

    #[test]
    fn split_message_should_split_at_3000_characters_when_the_limit_is_the_default() {
        assert_eq!(MAX_MESSAGE_CHARS, 3000);
        let text = "x".repeat(3000);
        assert_eq!(split_message(&text, MAX_MESSAGE_CHARS).len(), 1);
        assert_eq!(
            split_message(&format!("{text}x"), MAX_MESSAGE_CHARS).len(),
            2
        );
    }
}
