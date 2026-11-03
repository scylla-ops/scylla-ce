//! The Apps feature: the machine identities of a workspace with their Ed25519 keys and join
//! codes, `AppService`, and the check of a signed assertion.

mod assertion;
mod convert;
mod grpc;
mod keys;
mod plugin;
mod service;
mod store;

pub use assertion::{verify as verify_assertion, AUDIENCE};
pub(crate) use plugin::Apps;
pub(crate) use service::{hello, runner, touch, workspace_of};
pub use service::{join, JOIN_CODE_TTL};
