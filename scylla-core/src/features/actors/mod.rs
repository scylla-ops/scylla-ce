//! The actors feature: the `actors` table, and the identity relationship of each actor.

mod service;
mod store;

pub use service::{create, find};
pub(crate) use store::insert;
