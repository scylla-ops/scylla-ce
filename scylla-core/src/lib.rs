//! The core of Scylla: the access gate, the API, the command line, the adapters, and the core
//! features grouped by subject.

pub mod access;
pub mod api;
mod app;
pub mod cli;
pub mod config;
mod error;
pub mod features;
pub mod infra;

pub use app::edition;
pub use config::{Config, SignUpMode};
pub use error::Error;
