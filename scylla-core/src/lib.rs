//! The core of Scylla: the core features, the server and the command line.

pub mod access;
mod app;
pub mod cli;
pub mod config;
pub mod crypto;
pub mod db;
mod error;
pub mod features;
pub mod mail;
pub mod parse;
pub mod secret;
pub mod server;

pub use app::edition;
pub use config::{Config, SignUpMode};
pub use error::Error;
