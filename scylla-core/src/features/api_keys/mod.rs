//! The API keys feature: secret keys that authenticate an actor, stored as hashes.

mod auth;
mod plugin;
mod service;
mod store;

pub(crate) use plugin::ApiKeys;
pub use service::{create, revoke, NewKey, HEADER};
