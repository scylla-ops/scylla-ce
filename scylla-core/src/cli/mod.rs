//! The command line of a binary: `serve` (the default), `migrate`, and the commands of the
//! operator.

mod accounts;
mod commands;
mod users;

pub use accounts::{ProviderCommand, SecretCommand};
pub use commands::{run, Cli, Command, TokenCommand};
pub use users::{AdminCommand, UserCommand};
