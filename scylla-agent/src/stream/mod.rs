//! The stream of the agent: the host that it tells, the samples of its machine, and one session
//! of the stream.

mod host;
mod machine;
mod session;

pub(crate) use host::hostname;
pub(crate) use machine::Machine;
pub(crate) use session::{session, Shared};
