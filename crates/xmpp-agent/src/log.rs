//! The operational log: one JSON object per line. Starts, stops, approvals asked and decided, errors. Never a secret, never a message body.

use std::path::Path;

use serde_json::json;
use xmpp_agent_common::redact::redact;

/// Longest error text in the log, in characters.
pub const ERROR_MAX_CHARS: usize = 200;

/// Something worth a log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvent {
    /// The loop started.
    Started,
    /// The loop ended.
    Stopped,
    /// An agent session is up.
    AgentStarted {
        /// The ACP session id.
        session_id: String,
    },
    /// The agent process ended.
    AgentExited {
        /// What the adapter reported.
        reason: String,
    },
    /// An approval was put to the approvers.
    ApprovalAsked {
        /// The broker's request id.
        request_id: String,
        /// The tool's name.
        tool: String,
    },
    /// An approval was decided.
    ApprovalDecided {
        /// The broker's request id.
        request_id: String,
        /// The tool's name.
        tool: String,
        /// `allow` or `deny`.
        behavior: String,
        /// Why: approved, denied, timeout, shutdown, ...
        reason: String,
    },
    /// A message from a JID that may not talk to the agent was dropped.
    IgnoredSender {
        /// The sender's bare JID.
        from: String,
    },
    /// Something failed.
    Error {
        /// What failed. Redacted and cut by the formatter.
        what: String,
    },
}

/// Format one log line (no trailing newline). `ts_ms` is the time in milliseconds since the Unix epoch.
pub fn jsonl(ts_ms: u64, event: &LogEvent) -> String {
    let clean = |text: &str| {
        redact(text)
            .chars()
            .take(ERROR_MAX_CHARS)
            .collect::<String>()
    };
    let value = match event {
        LogEvent::Started => json!({"ts": ts_ms, "event": "started"}),
        LogEvent::Stopped => json!({"ts": ts_ms, "event": "stopped"}),
        LogEvent::AgentStarted { session_id } => {
            json!({"ts": ts_ms, "event": "agent_started", "session_id": clean(session_id)})
        }
        LogEvent::AgentExited { reason } => {
            json!({"ts": ts_ms, "event": "agent_exited", "reason": clean(reason)})
        }
        LogEvent::ApprovalAsked { request_id, tool } => {
            json!({"ts": ts_ms, "event": "approval_asked", "request_id": request_id, "tool": clean(tool)})
        }
        LogEvent::ApprovalDecided {
            request_id,
            tool,
            behavior,
            reason,
        } => {
            json!({"ts": ts_ms, "event": "approval_decided", "request_id": request_id, "tool": clean(tool), "behavior": behavior, "reason": reason})
        }
        LogEvent::IgnoredSender { from } => {
            json!({"ts": ts_ms, "event": "ignored_sender", "from": from})
        }
        LogEvent::Error { what } => json!({"ts": ts_ms, "event": "error", "what": clean(what)}),
    };
    value.to_string()
}

/// Where log events go.
pub trait Log: Send + Sync + 'static {
    /// Record one event. A failure to write is not an error of the caller.
    fn record(&self, event: &LogEvent);
}

/// Append JSONL to a file.
#[derive(Debug)]
pub struct FileLog {
    path: std::path::PathBuf,
}

impl FileLog {
    /// Log to this path; the file is created on the first event.
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
        }
    }
}

impl Log for FileLog {
    fn record(&self, event: &LogEvent) {
        use std::io::Write;
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(0));
        let line = jsonl(ts, event);
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{line}");
        }
    }
}

/// Log nothing.
#[derive(Debug, Clone, Copy)]
pub struct NoLog;

impl Log for NoLog {
    fn record(&self, _event: &LogEvent) {}
}

#[cfg(test)]
mod lines {
    use serde_json::{Value, json};

    use super::*;

    fn parse(ts: u64, event: &LogEvent) -> Value {
        let line = jsonl(ts, event);
        assert!(!line.contains('\n'), "{line:?}");
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("{line:?}: {e}"))
    }

    #[test]
    fn jsonl_should_carry_the_time_and_the_event_name_when_the_event_has_no_fields() {
        assert_eq!(
            parse(1700, &LogEvent::Started),
            json!({"ts": 1700, "event": "started"})
        );
        assert_eq!(
            parse(1701, &LogEvent::Stopped),
            json!({"ts": 1701, "event": "stopped"})
        );
    }

    #[test]
    fn jsonl_should_carry_the_fields_of_each_event() {
        let started = parse(
            1,
            &LogEvent::AgentStarted {
                session_id: "s-1".to_owned(),
            },
        );
        assert_eq!(
            started,
            json!({"ts": 1, "event": "agent_started", "session_id": "s-1"})
        );
        let asked = parse(
            2,
            &LogEvent::ApprovalAsked {
                request_id: "a1".to_owned(),
                tool: "Bash".to_owned(),
            },
        );
        assert_eq!(
            asked,
            json!({"ts": 2, "event": "approval_asked", "request_id": "a1", "tool": "Bash"})
        );
        let decided = parse(
            3,
            &LogEvent::ApprovalDecided {
                request_id: "a1".to_owned(),
                tool: "Bash".to_owned(),
                behavior: "deny".to_owned(),
                reason: "timeout".to_owned(),
            },
        );
        assert_eq!(
            decided,
            json!({"ts": 3, "event": "approval_decided", "request_id": "a1", "tool": "Bash", "behavior": "deny", "reason": "timeout"})
        );
        let ignored = parse(
            4,
            &LogEvent::IgnoredSender {
                from: "bob@chat.example.org".to_owned(),
            },
        );
        assert_eq!(
            ignored,
            json!({"ts": 4, "event": "ignored_sender", "from": "bob@chat.example.org"})
        );
    }

    #[test]
    fn jsonl_should_redact_and_cut_an_error_text() {
        let long = format!("failed with token=abcdef123456 {}", "x".repeat(500));
        let value = parse(5, &LogEvent::Error { what: long });
        let what = value["what"].as_str().expect("text");
        assert_eq!(value["event"], "error");
        assert!(!what.contains("abcdef123456"), "{what}");
        assert_eq!(what.chars().count(), ERROR_MAX_CHARS);
    }

    #[test]
    fn jsonl_should_redact_the_reason_when_the_agent_exits() {
        let value = parse(
            6,
            &LogEvent::AgentExited {
                reason: "the agent exited: password=hunter2value".to_owned(),
            },
        );
        assert_eq!(value["event"], "agent_exited");
        assert!(
            !value["reason"]
                .as_str()
                .expect("text")
                .contains("hunter2value")
        );
    }

    #[test]
    fn jsonl_should_keep_one_line_when_a_text_has_a_newline() {
        let value = parse(
            7,
            &LogEvent::Error {
                what: "first\nsecond".to_owned(),
            },
        );
        assert_eq!(value["what"], "first\nsecond");
    }

    #[test]
    fn file_log_should_append_one_line_per_event() {
        let dir = std::env::temp_dir().join(format!("xat-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("ops.jsonl");
        let log = FileLog::new(&path);
        log.record(&LogEvent::Started);
        log.record(&LogEvent::Stopped);
        let text = std::fs::read_to_string(&path).expect("log file");
        std::fs::remove_dir_all(&dir).expect("clean");
        let events: Vec<String> = text
            .lines()
            .map(|l| {
                serde_json::from_str::<Value>(l).expect("JSON")["event"]
                    .as_str()
                    .expect("name")
                    .to_owned()
            })
            .collect();
        assert_eq!(events, ["started", "stopped"]);
    }
}
