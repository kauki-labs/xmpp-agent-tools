//! What the loop says in chat, and how it reads the agent's permission options. Pure.

use serde_json::Value;
use xmpp_agent_acp::PermOption;
use xmpp_agent_client::router::{Reason, Rejected};
use xmpp_agent_common::redact::describe_tool;

use crate::split::{MAX_MESSAGE_CHARS, split_message};

/// Longest tool line, in characters.
pub const TOOL_LINE_MAX_CHARS: usize = 200;

/// The help text of `/help`.
pub const HELP: &str = "Commands: /new starts a fresh session, /stop cancels the turn, /status shows the state, /help shows this. Other /commands go to the agent when it has them.";

/// One short line for a tool call: the redacted description, never the tool's output.
pub fn tool_line(title: &str, raw_input: &Value) -> String {
    let described = describe_tool(title, raw_input, TOOL_LINE_MAX_CHARS);
    format!("tool: {}", described.replace(['\n', '\r'], " "))
}

/// The messages that end a turn: the agent's text split for chat, or a short note when the turn said nothing.
pub fn turn_messages(text: &str, stop_reason: &str) -> Vec<String> {
    let mut messages = split_message(text, MAX_MESSAGE_CHARS);
    let cancelled = stop_reason == "cancelled";
    if messages.is_empty() && !cancelled {
        messages.push("The agent finished without a message.".to_owned());
    }
    if cancelled {
        messages.push("Stopped.".to_owned());
    }
    messages
}

/// The id of the option that allows (or refuses) a call: once first, then always. `None` when the agent offers neither.
pub fn pick_option(options: &[PermOption], allow: bool) -> Option<String> {
    let (once, always) = if allow {
        ("allow_once", "allow_always")
    } else {
        ("reject_once", "reject_always")
    };
    [once, always]
        .iter()
        .find_map(|kind| options.iter().find(|option| option.kind == *kind))
        .map(|option| option.id.clone())
}

/// The name of a tool as the approvers see it: the title, or a placeholder when the agent sent none.
pub fn display_tool(title: &str) -> String {
    title.to_owned()
}

/// Make the broker's approval texts true for this loop, which never remembers a call: option 2 allows once.
pub fn honest_approval_text(text: &str) -> String {
    text.to_owned()
}

/// The prompt sent to the agent after an approval timed out, so it can continue with independent work.
pub fn blocked_prompt(tool: &str) -> String {
    format!(
        "The step \"{tool}\" is blocked on a human who has not answered, so it was denied. Do not retry it. Continue with independent work, \
         and say what you need from the human when you are done."
    )
}

/// The prompt that tells the agent why a call was denied with a note.
pub fn denied_prompt(tool: &str, note: &str) -> String {
    format!("The operator denied the step \"{tool}\": {note} Do not retry it; continue without it.")
}

/// The one-line reply to a rejected message; `None` for an empty message.
pub fn rejection_text(rejected: &Rejected) -> Option<String> {
    let name = &rejected.detail;
    match rejected.reason {
        Reason::Empty => None,
        Reason::Interactive => Some(format!("/{name} only works in a terminal.")),
        Reason::Unknown if name.is_empty() => {
            Some("Unknown command. Send /help for the commands.".to_owned())
        }
        Reason::Unknown if rejected.suggestions.is_empty() => Some(format!(
            "Unknown command /{name}. Send /help for the commands."
        )),
        Reason::Unknown => {
            let suggestions: Vec<String> = rejected
                .suggestions
                .iter()
                .map(|s| format!("/{s}"))
                .collect();
            Some(format!(
                "Unknown command /{name}. Did you mean {}?",
                suggestions.join(", ")
            ))
        }
    }
}

/// The text of `/status`.
pub fn status_text(
    running: bool,
    busy: bool,
    session_id: Option<&str>,
    pending_approvals: usize,
) -> String {
    let state = match (running, busy) {
        (false, _) => "stopped. Send /new to start a new session".to_owned(),
        (true, true) => "running, busy".to_owned(),
        (true, false) => "running, idle".to_owned(),
    };
    let session = session_id
        .map(|id| format!(" Session: {id}."))
        .unwrap_or_default();
    format!("Agent: {state}.{session} Approvals waiting: {pending_approvals}.")
}

#[cfg(test)]
mod text {
    use serde_json::json;
    use xmpp_agent_client::router::{Reason, Rejected};

    use super::*;

    fn option(id: &str, kind: &str) -> PermOption {
        PermOption {
            id: id.to_owned(),
            name: id.to_owned(),
            kind: kind.to_owned(),
        }
    }

    fn all_options() -> Vec<PermOption> {
        vec![
            option("ao", "allow_once"),
            option("aa", "allow_always"),
            option("ro", "reject_once"),
            option("ra", "reject_always"),
        ]
    }

    #[test]
    fn tool_line_should_show_the_tool_and_its_input_in_one_line() {
        let line = tool_line("Bash", &json!({"command": "ls -la"}));
        assert!(line.starts_with("tool: "), "{line}");
        assert!(line.contains("ls -la"), "{line}");
        assert!(!line.contains('\n'), "{line}");
    }

    #[test]
    fn tool_line_should_redact_a_secret_in_the_input() {
        let line = tool_line(
            "Bash",
            &json!({"command": "curl -H 'Authorization: Bearer abcdef0123456789xyz' https://example.org"}),
        );
        assert!(!line.contains("abcdef0123456789xyz"), "{line}");
    }

    #[test]
    fn tool_line_should_cut_a_long_input() {
        let line = tool_line("Bash", &json!({"command": "x".repeat(2000)}));
        assert!(
            line.chars().count() <= TOOL_LINE_MAX_CHARS + "tool: ".len(),
            "{}",
            line.chars().count()
        );
    }

    #[test]
    fn tool_line_should_not_show_file_content_when_the_tool_writes_a_file() {
        let line = tool_line(
            "Write",
            &json!({"file_path": "/home/user/a.txt", "content": "top secret body"}),
        );
        assert!(!line.contains("top secret body"), "{line}");
    }

    #[test]
    fn turn_messages_should_return_the_text_when_the_turn_has_text() {
        assert_eq!(turn_messages("hello", "end_turn"), ["hello"]);
    }

    #[test]
    fn turn_messages_should_split_a_long_text_at_3000_characters() {
        let text = (0..600)
            .map(|n| format!("line {n} of the answer"))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = turn_messages(&text, "end_turn");
        assert!(messages.len() > 1);
        assert!(messages.iter().all(|m| m.chars().count() <= 3000));
    }

    #[test]
    fn turn_messages_should_say_the_agent_stopped_when_the_turn_was_cancelled() {
        assert_eq!(turn_messages("", "cancelled"), ["Stopped."]);
        assert_eq!(
            turn_messages("partial", "cancelled"),
            ["partial", "Stopped."]
        );
    }

    #[test]
    fn turn_messages_should_say_so_when_a_finished_turn_had_no_text() {
        assert_eq!(
            turn_messages("  \n", "end_turn"),
            ["The agent finished without a message."]
        );
    }

    #[test]
    fn pick_option_should_prefer_once_over_always() {
        assert_eq!(pick_option(&all_options(), true), Some("ao".to_owned()));
        assert_eq!(pick_option(&all_options(), false), Some("ro".to_owned()));
    }

    #[test]
    fn pick_option_should_fall_back_to_reject_always_but_never_to_allow_always() {
        let options = vec![option("aa", "allow_always"), option("ra", "reject_always")];
        assert_eq!(pick_option(&options, true), None);
        assert_eq!(pick_option(&options, false), Some("ra".to_owned()));
    }

    #[test]
    fn turn_messages_should_report_a_failed_turn_when_the_stop_reason_is_an_error() {
        assert_eq!(
            turn_messages("", "error: model overloaded"),
            ["The turn failed: model overloaded"]
        );
        assert_eq!(
            turn_messages("partial", "error: boom"),
            ["partial", "The turn failed: boom"]
        );
    }

    #[test]
    fn turn_messages_should_redact_and_cut_the_error_of_a_failed_turn() {
        let reason = format!("error: bad token=abcdef123456 {}", "x".repeat(500));
        let messages = turn_messages("", &reason);
        assert!(!messages[0].contains("abcdef123456"), "{}", messages[0]);
        assert!(
            messages[0].chars().count() <= 220,
            "{}",
            messages[0].chars().count()
        );
    }

    #[test]
    fn display_tool_should_name_an_unnamed_tool() {
        assert_eq!(display_tool("touch a.txt"), "touch a.txt");
        assert_eq!(display_tool(""), "an unnamed tool");
        assert_eq!(display_tool("  "), "an unnamed tool");
    }

    #[test]
    fn honest_approval_text_should_say_option_2_allows_once() {
        let broker = "Approval #a1: touch a.txt\nReply 1 to allow once, 2 to allow this exact call again in this session, 3 to deny, or 4 and a note to deny with a reason. It is denied after 10 min without an answer.";
        let text = honest_approval_text(broker);
        assert!(!text.contains("again in this session"), "{text}");
        assert!(
            text.contains("2 to allow once")
                && text.contains("touch a.txt")
                && text.contains("3 to deny"),
            "{text}"
        );
    }

    #[test]
    fn honest_approval_text_should_correct_the_notice_of_answer_2() {
        let text = honest_approval_text("Allowed #a1, and the same call again in this session.");
        assert!(!text.contains("again in this session"), "{text}");
        assert!(
            text.contains("Allowed #a1") && text.contains("once"),
            "{text}"
        );
    }

    #[test]
    fn honest_approval_text_should_leave_other_text_alone() {
        assert_eq!(honest_approval_text("Denied #a1."), "Denied #a1.");
    }

    #[test]
    fn pick_option_should_never_answer_with_the_opposite_kind() {
        let only_allow = vec![option("ao", "allow_once"), option("aa", "allow_always")];
        assert_eq!(pick_option(&only_allow, false), None);
        let only_reject = vec![option("ro", "reject_once"), option("ra", "reject_always")];
        assert_eq!(pick_option(&only_reject, true), None);
        assert_eq!(pick_option(&[], true), None);
        assert_eq!(pick_option(&[option("x", "something_else")], false), None);
    }

    #[test]
    fn blocked_prompt_should_name_the_tool_and_ask_for_independent_work() {
        let text = blocked_prompt("touch a.txt");
        assert!(text.contains("touch a.txt"), "{text}");
        assert!(text.contains("human"), "{text}");
        assert!(text.contains("independent"), "{text}");
    }

    #[test]
    fn denied_prompt_should_carry_the_tool_and_the_note() {
        let text = denied_prompt("touch a.txt", "too risky");
        assert!(
            text.contains("touch a.txt") && text.contains("too risky"),
            "{text}"
        );
    }

    fn rejected(reason: Reason, detail: &str, suggestions: &[&str]) -> Rejected {
        Rejected {
            reason,
            detail: detail.to_owned(),
            suggestions: suggestions.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn rejection_text_should_stay_silent_when_the_message_was_empty() {
        assert_eq!(rejection_text(&rejected(Reason::Empty, "", &[])), None);
    }

    #[test]
    fn rejection_text_should_suggest_commands_when_the_name_is_close() {
        let text = rejection_text(&rejected(Reason::Unknown, "stats", &["status", "start"]))
            .expect("a reply");
        assert!(
            text.contains("/stats") && text.contains("/status") && text.contains("/start"),
            "{text}"
        );
        assert!(!text.contains('\n'), "{text}");
    }

    #[test]
    fn rejection_text_should_point_to_help_when_nothing_is_close() {
        let text = rejection_text(&rejected(Reason::Unknown, "zzz", &[])).expect("a reply");
        assert!(text.contains("/zzz") && text.contains("/help"), "{text}");
        let bare = rejection_text(&rejected(Reason::Unknown, "", &[])).expect("a reply");
        assert_eq!(bare, "Unknown command. Send /help for the commands.");
    }

    #[test]
    fn rejection_text_should_say_the_command_needs_a_terminal_when_it_is_interactive() {
        let text = rejection_text(&rejected(Reason::Interactive, "doctor", &[])).expect("a reply");
        assert!(
            text.contains("/doctor") && text.contains("terminal"),
            "{text}"
        );
    }

    #[test]
    fn status_text_should_report_the_state() {
        let text = status_text(true, false, Some("fake-1"), 2);
        assert!(
            text.contains("running")
                && text.contains("idle")
                && text.contains("fake-1")
                && text.contains('2'),
            "{text}"
        );
        let busy = status_text(true, true, Some("fake-1"), 0);
        assert!(busy.contains("busy"), "{busy}");
        let stopped = status_text(false, false, None, 0);
        assert!(
            stopped.contains("stopped") && stopped.contains("/new"),
            "{stopped}"
        );
    }

    #[test]
    fn help_should_list_the_commands_of_this_version() {
        for command in ["/new", "/stop", "/status", "/help"] {
            assert!(HELP.contains(command), "{command}");
        }
    }
}
