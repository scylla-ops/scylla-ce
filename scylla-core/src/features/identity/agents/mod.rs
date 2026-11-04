//! The agents feature: the stream of each App that runs jobs, `AgentService`, the jobs on the
//! stream, the job stats of the agents, and the registry of the open streams of this server.

mod grpc;
mod registry;
mod stats;
mod stream;
mod work;

pub(crate) use grpc::server;
pub(crate) use registry::Registry;
