//! The test kit: the real server of an edition, with a real gate, and helpers for the core RPCs.

mod app;

pub use app::{id, test_access, text, with_token, TestApp};
