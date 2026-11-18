//! The agents feature: the stream of each App that runs jobs, `AgentService`, the jobs on the
//! stream, the job stats and the busy slots of the agents, the samples of their machines, their
//! install scripts, and the registry of the open streams of this server.

mod dispatch;
mod grpc;
mod install;
mod registry;
mod samples;
mod stats;
mod store;
mod stream;
mod usage;
mod work;

#[cfg(test)]
mod tests;

pub(crate) use grpc::server;
pub(crate) use install::{router as install_router, AGENT_DOWNLOADS, AGENT_IMAGE};
pub(crate) use registry::{Features, Registry};
pub(crate) use samples::{last as last_samples, purge as purge_samples};
