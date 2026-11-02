//! The actors feature: the `actors` table, and the identity relation of each actor.

mod service;
mod store;

pub use service::{create, find, identity};
pub(crate) use store::insert;
