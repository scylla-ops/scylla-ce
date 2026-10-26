//! Named `rest`, not `http`: a crate-root `mod http` would shadow the external `http` crate.

pub mod health;
pub mod ui;
pub mod webhook;
