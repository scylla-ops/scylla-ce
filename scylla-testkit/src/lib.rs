//! The test kit: the real server of an edition, and helpers for the core RPCs.

mod app;

pub use app::{as_actor, id, text, with_key, TestApp};
