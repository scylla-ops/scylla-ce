//! The agents feature: the stream of each App that runs jobs, `AgentService`, and the registry of
//! the open streams of this server.

mod grpc;
mod registry;
mod stream;

pub(crate) use grpc::server;
pub(crate) use registry::Registry;
