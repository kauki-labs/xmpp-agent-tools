//! Agent Client Protocol (ACP) adapter: spawn an agent, speak JSON-RPC over its stdio, and turn what it sends into [`AgentEvent`]s.
//!
//! [`protocol`] is pure (decode and build messages, no I/O); [`session`] owns the process.

pub mod protocol;
pub mod session;

pub use protocol::{AgentEvent, Error, PermOption};
pub use session::{AcpSession, SpawnConfig};
