//! The agent of Scylla: it joins an App of a workspace with a key pair that it makes, keeps a
//! stream open with the server, and runs the jobs that the server gives it.

mod agent;
mod config;
mod identity;
mod jobs;
mod stream;

pub use agent::{run, AgentError, Settings};
pub use config::AgentConfig;
pub use identity::join;
