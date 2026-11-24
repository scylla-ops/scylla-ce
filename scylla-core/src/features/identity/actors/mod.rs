//! The actors feature: the `actors` table.

mod service;
mod store;

pub use service::{create, find};
pub(crate) use store::insert;
