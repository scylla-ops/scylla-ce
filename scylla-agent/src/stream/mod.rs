//! The stream of the agent: the host that it tells, and one session of the stream.

mod host;
mod session;

pub(crate) use session::{session, Shared};
