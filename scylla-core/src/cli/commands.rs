//! The parser and the runner of the commands. `run` runs one command with its settings.

use std::fmt;
use std::time::Duration;

use clap::{Parser, Subcommand};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::Id;
use scylla_extension::{Binding, Edition};

use super::accounts::{self, ProviderCommand, SecretCommand};
use super::users::{self, UserCommand};
use crate::access::{Access, TokenKeys};
use crate::features::identity::actors;
use crate::infra::{db, logs};
use crate::{api, Config, Error};

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
    /// Serves the gRPC and gRPC-Web API until SIGTERM or Ctrl+C.
    #[default]
    Serve,
    /// Applies the migrations and writes the SpiceDB schema, then stops.
    Migrate,
    /// Manages the users.
    #[command(subcommand)]
    User(UserCommand),
    /// Manages the identity providers of the sign-in.
    #[command(subcommand)]
    Provider(ProviderCommand),
    /// Manages the tokens.
    #[command(subcommand)]
    Token(TokenCommand),
    /// Manages the key of the secrets: the client secrets of the identity providers and the
    /// secrets of the projects.
    #[command(subcommand)]
    Secret(SecretCommand),
}

/// The commands on the tokens.
#[derive(Subcommand, Clone, Debug)]
pub enum TokenCommand {
    /// Prints a new signing key pair, to store as `SCYLLA_TOKEN_PRIVATE_KEY`.
    Keygen,
    /// Signs a token for an actor, for a first admin or an emergency. Prints it once.
    Create {
        /// The id of the actor.
        #[arg(long, value_parser = id::<Actor>)]
        actor: ActorId,
        /// The lifetime, in seconds. The default is the token lifetime of the settings.
        #[arg(long)]
        ttl: Option<u64>,
    },
    /// Revokes a token and each token made from it.
    Revoke {
        /// The token.
        token: String,
    },
}

/// Starts the logs, then runs the command.
pub async fn run<C>(config: &C, command: Command, edition: Edition) -> Result<(), Error>
where
    C: AsRef<Config> + fmt::Debug,
{
    // Install the crypto provider and start the logs.
    let core = config.as_ref();
    crate::infra::tls::install();
    logs::init(core.log_format);
    tracing::info!(edition = edition.name(), ?command, ?config, "starting");

    // Run the command with the core settings.
    match command {
        Command::Serve => api::run(core, edition).await,
        Command::Migrate => migrate(core, &edition).await,
        Command::User(command) => users::user(core, command).await,
        Command::Provider(command) => accounts::provider(core, command).await,
        Command::Secret(command) => {
            accounts::secret(command);
            Ok(())
        }
        Command::Token(command) => token(core, command, &edition).await,
    }
}

/// Applies the migrations of the edition, then writes its SpiceDB schema.
async fn migrate(config: &Config, edition: &Edition) -> Result<(), Error> {
    // Apply the migrations of each migrator, in order.
    let pool = db::connect(config).await?;
    db::migrate(&pool, edition.migrators()).await?;

    // Write the SpiceDB schema of the edition.
    let access = Access::from_config(config, pool.clone(), edition.authz_model().clone())?;
    access
        .spicedb()
        .write_schema(edition.authz_schema())
        .await?;
    pool.close().await;
    tracing::info!("migrations and schema applied");
    Ok(())
}

/// Runs a token command.
async fn token(config: &Config, command: TokenCommand, edition: &Edition) -> Result<(), Error> {
    // Make a key pair. It needs no database and no key.
    if let TokenCommand::Keygen = command {
        let keys = TokenKeys::generate(config.token_key_id);
        println!("private: {}", keys.private_hex());
        println!("public: {}:{}", config.token_key_id, keys.public_hex());
        return Ok(());
    }

    // Sign or revoke a token with the gate of the settings.
    let pool = db::connect(config).await?;
    let access = Access::from_config(config, pool.clone(), edition.authz_model().clone())?;
    match command {
        TokenCommand::Keygen => {}
        TokenCommand::Create { actor, ttl } => {
            let actor = actors::find(&pool, actor).await?;
            let ttl = ttl.map_or(config.token_ttl, Duration::from_secs);
            let token = access.issue_for(&actor, ttl, Binding::None)?;
            println!("{}", token.value);
        }
        TokenCommand::Revoke { token } => {
            for id in access.revoke(&token).await? {
                println!("revoked: {id}");
            }
        }
    }
    pool.close().await;
    Ok(())
}

/// Reads an id from the command line.
pub(super) fn id<T: ?Sized + 'static>(text: &str) -> Result<Id<T>, String> {
    Id::parse(text).map_err(|error| error.to_string())
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
