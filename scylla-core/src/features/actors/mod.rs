//! The actors feature: the `actors` table, and the dev authenticator.

mod header;
mod service;
mod store;

pub use header::{TrustedHeader, ACTOR_HEADER};
pub use service::create;
