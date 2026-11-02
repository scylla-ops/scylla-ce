//! The command line of a binary: `serve` (the default), `migrate`, and the commands of the
//! operator.

mod accounts;
mod commands;

pub use accounts::{ProviderCommand, SecretCommand, UserCommand};
pub use commands::{run, Cli, Command, TokenCommand};
