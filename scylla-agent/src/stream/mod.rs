//! The stream of the agent: the host that it tells, its protocol, and one session of the stream.

mod host;
mod protocol;
mod session;

pub(crate) use host::hostname;
pub use protocol::AGENT_PROTOCOL;
pub(crate) use session::{session, Shared};
