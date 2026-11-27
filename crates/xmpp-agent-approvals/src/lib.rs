//! Approvals and dialogs for a harness. Pure logic: the clock and the id source are arguments, so tests need no sleeps.
//!
//! Port of `src/xmpp_agent_client/approvals.py`. The harness blocks on `--permission-prompt-tool` until a decision comes back
//! (docs/experiments.md E3: it waited 10.5 minutes without a timeout). The [`Broker`] decides what the answer is. Rules:
//!
//! - Only an approver's real bare JID counts. A MUC nickname is not a JID, so the bridge must pass the occupant's real JID.
//! - Fail closed: a timeout, a shutdown, too many pending requests, a malformed question and a stranger's reply all deny or change
//!   nothing.
//! - A request resolves at most once. Replies are matched by context (room and thread), and by `#id` when several are pending.
//! - Ordinary chat is never mistaken for a decision: only "1", "2", "3", "4 note", "y", "yes", "allow", "ok", "n", "no", "deny" count.
//!
//! Text handling follows Python, not Rust, wherever the two differ (whitespace, case folding, digits): see [`text`].

pub mod broker;
mod text;

pub use broker::{
    Behavior, Broker, DEFAULT_MAX_PENDING, DEFAULT_TIMEOUT, DENY_MESSAGE, Decision,
    IdAllocationError, Reason, Reply, ReplyKind, Request, RequestKind, Submitted,
};
