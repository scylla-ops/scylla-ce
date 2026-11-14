//! The agents feature: the stream of each App that runs jobs, `AgentService`, the jobs on the
//! stream, the job stats of the agents, their install scripts, and the registry of the open
//! streams of this server.

mod dispatch;
mod grpc;
mod install;
mod registry;
mod stats;
mod stream;
mod work;

pub(crate) use grpc::server;
pub(crate) use install::{router as install_router, AGENT_DOWNLOADS, AGENT_IMAGE};
pub(crate) use registry::Registry;
