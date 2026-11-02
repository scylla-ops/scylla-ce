//! The agents feature: agents with an Ed25519 key pair, and `AgentService` to register them.

mod grpc;
mod plugin;
mod service;
mod store;

pub(crate) use plugin::Agents;
pub use service::{register, verify_assertion, AUDIENCE};
