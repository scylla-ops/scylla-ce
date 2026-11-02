//! The `scylla-ce` binary. Run `scylla-ce --help` for the commands and the settings.

use clap::Parser;
use scylla_core::cli::{self, Cli};

/// Reads the command line, then runs the command.
#[tokio::main]
async fn main() -> Result<(), scylla_core::Error> {
    let cli = Cli::parse();
    cli::run(
        &cli.config,
        cli.command.unwrap_or_default(),
        scylla_ce::edition(),
    )
    .await
}
