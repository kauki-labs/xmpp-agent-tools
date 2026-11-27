//! Text rules that follow Python's `str` and `re`, not Rust's.
//!
//! The broker decides from chat text, so a character that one language treats as whitespace or as a digit and the other does not
//! would change a decision. Each rule here was checked against Python 3.13 (Unicode 15.1):
//!
//! - Whitespace is `str.isspace`: Rust's `White_Space` plus U+001C to U+001F.
//! - A digit is a Unicode decimal digit (`\d` in a `str` pattern, and what `int()` accepts), the table below. It is a table, not the
//!   `regex` crate's `\d`, so the digit set cannot move with a Unicode update on one side only.
//! - Lowercasing is `str.lower`, which Rust's `to_lowercase` matches (full mappings and the final sigma rule).

use serde_json::Value;

/// Code points of the digit zero of each run of ten Unicode decimal digits (category Nd), in Python 3.13's Unicode 15.1.
const DIGIT_ZEROS: [u32; 68] = [
    0x30, 0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66,
    0xDE6, 0xE50, 0xED0, 0xF20, 0x1040, 0x1090, 0x17E0, 0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90,
    0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10,
    0x104A0, 0x10D30, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0, 0x11650,
    0x116C0, 0x11730, 0x118E0, 0x11950, 0x11C50, 0x11D50, 0x11DA0, 0x11F50, 0x16A60, 0x16AC0,
    0x16B50, 0x1D7CE, 0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6, 0x1E140, 0x1E2F0, 0x1E4F0, 0x1E950,
    0x1FBF0,
];

/// Whether Python's `str.isspace` is true for the character.
pub(crate) fn is_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()` without arguments.
pub(crate) fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// Python's `str.split(None, 1)`: the first word and the rest with its leading whitespace removed, or `None` when there is no word.
pub(crate) fn split_first_word(text: &str) -> Option<(&str, &str)> {
    let text = text.trim_start_matches(is_space);
    if text.is_empty() {
        return None;
    }
    match text.find(is_space) {
        Some(end) => Some((&text[..end], text[end..].trim_start_matches(is_space))),
        None => Some((text, "")),
    }
}

/// Python's `str.lower()`.
pub(crate) fn lower(text: &str) -> String {
    text.to_lowercase()
}

/// Whether Python treats the value as true: not null, false, zero, or an empty string, array or object.
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(members) => !members.is_empty(),
    }
}

/// The value of a Unicode decimal digit, or `None` for any other character.
fn digit_value(c: char) -> Option<usize> {
    let code = u32::from(c);
    DIGIT_ZEROS
        .iter()
        .find(|zero| (**zero..**zero + 10).contains(&code))
        .map(|zero| (code - zero) as usize)
}

/// The numbers of a text that `\d+(?:\s*[,\s]\s*\d+)*` matches in full, read as `int()` reads them.
///
/// `None` when the text does not match. A number too large for `usize` saturates: it is above any option count either way. (Python
/// 3.13 raises `ValueError` for a number of more than 4300 digits; the port rejects it as out of range.)
pub(crate) fn parse_numbers(text: &str) -> Option<Vec<usize>> {
    let mut numbers = Vec::new();
    let mut rest = text;
    loop {
        let digits = rest
            .find(|c| digit_value(c).is_none())
            .unwrap_or(rest.len());
        if digits == 0 {
            return None;
        }
        let value = rest[..digits]
            .chars()
            .filter_map(digit_value)
            .fold(0usize, |sum, digit| {
                sum.saturating_mul(10).saturating_add(digit)
            });
        numbers.push(value);
        rest = &rest[digits..];
        if rest.is_empty() {
            return Some(numbers);
        }
        // Between two numbers: whitespace around at most one comma, and at least one character.
        let gap = rest
            .find(|c: char| !is_space(c) && c != ',')
            .unwrap_or(rest.len());
        if gap == rest.len() || rest[..gap].matches(',').count() > 1 {
            return None;
        }
        rest = &rest[gap..];
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn is_space_should_match_python_when_the_character_is_one_python_calls_whitespace() {
        // Python: [hex(c) for c in range(0x110000) if chr(c).isspace()]
        let python = [
            0x9, 0xa, 0xb, 0xc, 0xd, 0x1c, 0x1d, 0x1e, 0x1f, 0x20, 0x85, 0xa0, 0x1680, 0x2000,
            0x2001, 0x2002, 0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028,
            0x2029, 0x202f, 0x205f, 0x3000,
        ];
        let rust: Vec<u32> = (0..0x11_0000)
            .filter_map(char::from_u32)
            .filter(|c| is_space(*c))
            .map(u32::from)
            .collect();
        assert_eq!(rust, python);
    }

    #[test]
    fn strip_should_remove_python_whitespace_when_it_surrounds_the_text() {
        assert_eq!(strip("\u{1f} x \u{1c}"), "x");
        assert_eq!(strip("\u{85}x\u{a0}"), "x");
        assert_eq!(strip("\u{3000}x y\u{2029}"), "x y");
        assert_eq!(strip(""), "");
        assert_eq!(strip(" \t\n"), "");
    }

    #[test]
    fn strip_should_keep_characters_that_are_not_whitespace_in_python() {
        assert_eq!(strip("\u{200b}x\u{200b}"), "\u{200b}x\u{200b}");
        assert_eq!(strip("\u{feff}x"), "\u{feff}x");
        assert_eq!(strip("\u{180e}x"), "\u{180e}x");
    }

    #[test]
    fn split_first_word_should_split_on_the_first_whitespace_run_when_there_are_two_words() {
        assert_eq!(split_first_word("no thanks"), Some(("no", "thanks")));
        assert_eq!(split_first_word("a\u{1f}b c"), Some(("a", "b c")));
        assert_eq!(split_first_word("  a \u{a0} b  "), Some(("a", "b  ")));
    }

    #[test]
    fn split_first_word_should_give_an_empty_rest_when_there_is_one_word() {
        assert_eq!(split_first_word("yes"), Some(("yes", "")));
        assert_eq!(split_first_word("\u{1f}yes\u{85}"), Some(("yes", "")));
    }

    #[test]
    fn split_first_word_should_return_none_when_there_is_no_word() {
        assert_eq!(split_first_word(""), None);
        assert_eq!(split_first_word(" \u{1f}\t"), None);
    }

    #[test]
    fn lower_should_match_python_when_the_text_is_not_ascii() {
        assert_eq!(lower("ME@Chat.Example"), "me@chat.example");
        assert_eq!(lower("\u{130}"), "i\u{307}");
        assert_eq!(lower("\u{391}\u{3a3}"), "\u{3b1}\u{3c2}");
        assert_eq!(lower("\u{391}\u{3a3} \u{392}"), "\u{3b1}\u{3c2} \u{3b2}");
        assert_eq!(lower("\u{1e9e}"), "\u{df}");
        assert_eq!(lower("\u{212a}"), "k");
    }

    #[test]
    fn truthy_should_follow_python_when_the_value_is_empty_or_zero() {
        for empty in [
            json!(null),
            json!(false),
            json!(0),
            json!(0.0),
            json!(""),
            json!([]),
            json!({}),
        ] {
            assert!(!truthy(&empty), "{empty}");
        }
    }

    #[test]
    fn truthy_should_follow_python_when_the_value_has_content() {
        for full in [
            json!(true),
            json!(1),
            json!(-1),
            json!(0.5),
            json!("no"),
            json!("0"),
            json!([0]),
            json!({"a": null}),
        ] {
            assert!(truthy(&full), "{full}");
        }
    }

    #[test]
    fn parse_numbers_should_read_numbers_when_a_comma_or_whitespace_separates_them() {
        // Python: bool(re.fullmatch(r"\d+(?:\s*[,\s]\s*\d+)*", text)) and the int() of each run of digits.
        assert_eq!(parse_numbers("2"), Some(vec![2]));
        assert_eq!(parse_numbers("1,2"), Some(vec![1, 2]));
        assert_eq!(parse_numbers("1 , 2"), Some(vec![1, 2]));
        assert_eq!(parse_numbers("1, 2 ,3"), Some(vec![1, 2, 3]));
        assert_eq!(parse_numbers("1 2"), Some(vec![1, 2]));
        assert_eq!(parse_numbers("1,\n2"), Some(vec![1, 2]));
        assert_eq!(parse_numbers("1\u{1c}2"), Some(vec![1, 2]));
        assert_eq!(parse_numbers("1\u{a0}2"), Some(vec![1, 2]));
        assert_eq!(parse_numbers("007"), Some(vec![7]));
    }

    #[test]
    fn parse_numbers_should_return_none_when_the_text_is_not_only_numbers() {
        for text in [
            "", ",1", "1,", "1,,2", "1 ,, 2", "1.2", "1a", "a1", "-1", "+1", " 1", "1 ", "\u{66b}",
        ] {
            assert_eq!(parse_numbers(text), None, "{text:?}");
        }
    }

    #[test]
    fn parse_numbers_should_read_unicode_decimal_digits_when_the_text_has_them() {
        assert_eq!(parse_numbers("\u{663},\u{662}"), Some(vec![3, 2]));
        assert_eq!(parse_numbers("\u{ff11}\u{ff12}"), Some(vec![12]));
        assert_eq!(parse_numbers("\u{1d7ff}"), Some(vec![9]));
        assert_eq!(parse_numbers("1\u{966}"), Some(vec![10]));
    }

    #[test]
    fn parse_numbers_should_not_read_other_numerals_when_they_are_not_decimal_digits() {
        // U+00B2 superscript two (`isdigit` but not `isdecimal`), U+2167 Roman numeral eight, U+00BD vulgar fraction.
        for text in ["\u{b2}", "\u{2167}", "\u{bd}", "\u{2460}"] {
            assert_eq!(parse_numbers(text), None, "{text:?}");
        }
    }

    #[test]
    fn parse_numbers_should_saturate_when_a_number_is_larger_than_any_option_count() {
        let huge = "9".repeat(5000);
        assert_eq!(parse_numbers(&huge), Some(vec![usize::MAX]));
        assert_eq!(
            parse_numbers("99999999999999999999999,1"),
            Some(vec![usize::MAX, 1])
        );
    }

    #[test]
    fn digit_zeros_should_hold_every_python_decimal_digit_when_listed() {
        // Python: sum(1 for c in range(0x110000) if chr(c).isdecimal()) == 680, and every run is ten digits from value 0.
        let count = (0..0x11_0000)
            .filter_map(char::from_u32)
            .filter(|c| parse_numbers(&c.to_string()).is_some())
            .count();
        assert_eq!(count, 680);
        assert_eq!(DIGIT_ZEROS.len() * 10, 680);
    }
}
