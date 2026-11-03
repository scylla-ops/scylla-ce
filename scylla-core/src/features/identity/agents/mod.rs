//! The agents feature: the stream of each App that runs jobs, `AgentService`, the jobs on the
//! stream, and the registry of the open streams of this server.

mod grpc;
mod registry;
mod stream;
mod work;

pub(crate) use grpc::server;
pub(crate) use registry::Registry;
