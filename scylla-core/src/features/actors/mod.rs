//! The actors feature: the `actors` table, with the owner of each actor.

mod service;
mod store;

pub use service::{create, find};
pub(crate) use store::insert;
