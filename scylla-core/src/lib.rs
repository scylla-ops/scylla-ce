//! The core of Scylla: the core features, the server and the command line.

mod app;
pub mod cli;
pub mod config;
pub mod db;
mod error;
pub mod features;
pub mod parse;
pub mod server;

pub use app::edition;
pub use config::Config;
pub use error::Error;
