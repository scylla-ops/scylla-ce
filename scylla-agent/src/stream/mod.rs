//! The stream of the agent: the host that it tells, its protocol, the samples of its machine, and
//! one session of the stream.

mod host;
mod machine;
mod protocol;
mod session;

pub(crate) use host::hostname;
pub(crate) use machine::Machine;
pub use protocol::AGENT_PROTOCOL;
pub(crate) use session::{session, Shared};
