//! The Apps feature: the machine identities of a workspace with their Ed25519 keys, join codes
//! and API tokens, `AppService`, and the check of a signed assertion.

mod assertion;
mod convert;
mod grpc;
mod keys;
mod plugin;
mod service;
mod store;
mod token_service;
mod tokens;

pub use assertion::{verify as verify_assertion, AUDIENCE};
pub(crate) use convert::optional_label;
pub(crate) use grpc::server;
pub(crate) use plugin::Apps;
#[cfg(test)]
pub(crate) use service::create;
pub(crate) use service::{get, hello, runner, touch, workspace, workspace_of};
pub use service::{join, JOIN_CODE_TTL};
pub(crate) use token_service::exchange as exchange_token;
