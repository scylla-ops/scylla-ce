//! The command line of a binary: `serve` (the default), `migrate`, and the admin commands.

use std::fmt;

use clap::{Parser, Subcommand};
use scylla_domain::actors::{ActorId, ActorKind};
use scylla_domain::api_keys::ApiKeyId;
use scylla_domain::Id;
use scylla_extension::Edition;
use tracing::Level;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::features::{actors, api_keys};
use crate::{db, server, Config, Error};

/// The command line of a binary with only the core settings.
#[derive(Parser)]
#[command(version, about)]
pub struct Cli {
    /// The command to run.
    #[command(subcommand)]
    pub command: Option<Command>,

    /// The settings.
    #[command(flatten)]
    pub config: Config,
}

/// The commands of a binary.
#[derive(Subcommand, Clone, Debug, Default)]
pub enum Command {
    /// Serves the gRPC API until SIGTERM or Ctrl+C.
    #[default]
    Serve,
    /// Applies the core migrations, then the edition migrations, then stops.
    Migrate,
    /// Manages the actors.
    #[command(subcommand)]
    Actor(ActorCommand),
    /// Manages the API keys.
    #[command(subcommand)]
    Key(KeyCommand),
}

/// The commands on the actors.
#[derive(Subcommand, Clone, Debug)]
pub enum ActorCommand {
    /// Creates an actor and prints its id.
    Create {
        /// The kind of the actor, in snake case.
        #[arg(long)]
        kind: String,
    },
}

/// The commands on the API keys.
#[derive(Subcommand, Clone, Debug)]
pub enum KeyCommand {
    /// Creates an API key for an actor and prints it. The key is shown one time only.
    Create {
        /// The id of the actor.
        #[arg(long, value_parser = id::<scylla_domain::actors::Actor>)]
        actor: ActorId,
    },
    /// Revokes an API key.
    Revoke {
        /// The id of the key.
        #[arg(value_parser = id::<scylla_domain::api_keys::ApiKey>)]
        id: ApiKeyId,
    },
}

/// Starts the logs, then runs the command.
pub async fn run<C>(config: &C, command: Command, edition: Edition) -> Result<(), Error>
where
    C: AsRef<Config> + fmt::Debug,
{
    init_tracing();
    tracing::info!(edition = edition.name(), ?command, ?config, "starting");
    let core = config.as_ref();
    match command {
        Command::Serve => server::run(core, edition).await,
        Command::Migrate => migrate(core, &edition).await,
        Command::Actor(command) => actor(core, command).await,
        Command::Key(command) => key(core, command).await,
    }
}

/// Applies the migrations of the edition.
async fn migrate(config: &Config, edition: &Edition) -> Result<(), Error> {
    let pool = db::connect(config).await?;
    db::migrate(&pool, edition.migrators()).await?;
    pool.close().await;
    tracing::info!("migrations applied");
    Ok(())
}

/// Runs an actor command.
async fn actor(config: &Config, command: ActorCommand) -> Result<(), Error> {
    let pool = db::connect(config).await?;
    match command {
        ActorCommand::Create { kind } => {
            let actor = actors::create(&pool, ActorKind::from_name(kind)).await?;
            println!("{}", actor.id);
        }
    }
    pool.close().await;
    Ok(())
}

/// Runs a key command.
async fn key(config: &Config, command: KeyCommand) -> Result<(), Error> {
    let pool = db::connect(config).await?;
    match command {
        KeyCommand::Create { actor } => {
            let key = api_keys::create(&pool, actor).await?;
            println!("id: {}", key.id);
            println!("key: {}", key.secret);
        }
        KeyCommand::Revoke { id } => {
            api_keys::revoke(&pool, id).await?;
            println!("revoked: {id}");
        }
    }
    pool.close().await;
    Ok(())
}

/// Reads an id from the command line.
fn id<T: ?Sized + 'static>(text: &str) -> Result<Id<T>, String> {
    Id::parse(text).map_err(|error| error.to_string())
}

/// Starts the logs with the filter of `RUST_LOG`. The default hides the sqlx statement logs.
fn init_tracing() {
    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| value.parse::<Targets>().ok())
        .unwrap_or_else(|| {
            Targets::new()
                .with_default(Level::INFO)
                .with_target("sqlx", Level::WARN)
        });
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();
}

#[cfg(test)]
mod tests {
    //! Unit tests of the command line.

    use clap::CommandFactory;

    use super::Cli;

    /// The command line has no conflict, as clap checks it.
    #[test]
    fn cli_is_valid() {
        // given
        let command = Cli::command();

        // when
        // then
        command.debug_assert();
    }
}
