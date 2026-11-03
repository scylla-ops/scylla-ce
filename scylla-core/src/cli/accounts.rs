//! The commands of the operator on the sign-in: the identity providers, and the key of the
//! client secrets.

use std::io::BufRead;

use clap::Subcommand;
use scylla_domain::identity::users::ProviderId;

use crate::api::sealer;
use crate::features::identity::providers::{self, GitHubSettings, OidcSettings, ProviderRow};
use crate::infra::db;
use crate::infra::secret::Sealer;
use crate::{Config, Error};

/// The commands on the identity providers. A provider with the same slug is changed, and its
/// identities stay.
#[derive(Subcommand, Clone, Debug)]
pub enum ProviderCommand {
    /// Adds or changes an OpenID Connect provider.
    AddOidc {
        /// The key of the provider in the URLs.
        #[arg(long)]
        slug: String,
        /// The name to show.
        #[arg(long)]
        name: String,
        /// The issuer.
        #[arg(long)]
        issuer: String,
        /// The client id of Scylla at the issuer.
        #[arg(long)]
        client_id: String,
        /// Reads a client secret from the first line of stdin, for a confidential client.
        #[arg(long)]
        client_secret_stdin: bool,
        /// Lets a new person sign up with this provider.
        #[arg(long)]
        allow_sign_up: bool,
    },
    /// Adds or changes a GitHub provider. The client secret comes from the first line of stdin.
    AddGithub {
        /// The key of the provider in the URLs.
        #[arg(long)]
        slug: String,
        /// The name to show.
        #[arg(long)]
        name: String,
        /// The client id of the OAuth app.
        #[arg(long)]
        client_id: String,
        /// The web URL, for GitHub Enterprise Server.
        #[arg(long, default_value = "https://github.com")]
        web_url: String,
        /// The API URL, for GitHub Enterprise Server.
        #[arg(long, default_value = "https://api.github.com")]
        api_url: String,
        /// Lets a new person sign up with this provider.
        #[arg(long)]
        allow_sign_up: bool,
    },
    /// Lists the providers.
    List,
    /// Turns a provider off. Its identities stay.
    Disable {
        /// The key of the provider.
        #[arg(long)]
        slug: String,
    },
}

/// The commands on the key of the client secrets.
#[derive(Subcommand, Clone, Debug)]
pub enum SecretCommand {
    /// Prints a new key, to store as `SCYLLA_SECRET_KEY`.
    Keygen,
}

/// Runs a provider command.
pub(super) async fn provider(config: &Config, command: ProviderCommand) -> Result<(), Error> {
    let pool = db::connect(config).await?;
    let mut tx = pool.begin().await?;
    match command {
        ProviderCommand::AddOidc {
            slug,
            name,
            issuer,
            client_id,
            client_secret_stdin,
            allow_sign_up,
        } => {
            let settings = OidcSettings {
                issuer,
                client_id,
                scopes: ["openid", "email", "profile"].map(str::to_owned).to_vec(),
            };
            let secret = match client_secret_stdin {
                true => Some(seal(config, &stdin_line()?)?),
                false => None,
            };
            let row = row(slug, "oidc", name, json(&settings)?, secret, allow_sign_up);
            providers::upsert(&mut tx, &row).await?;
        }
        ProviderCommand::AddGithub {
            slug,
            name,
            client_id,
            web_url,
            api_url,
            allow_sign_up,
        } => {
            let settings = GitHubSettings {
                client_id,
                web_url,
                api_url,
            };
            let secret = Some(seal(config, &stdin_line()?)?);
            let row = row(
                slug,
                "github",
                name,
                json(&settings)?,
                secret,
                allow_sign_up,
            );
            providers::upsert(&mut tx, &row).await?;
        }
        ProviderCommand::List => {
            for provider in providers::list(&mut tx, false).await? {
                let ProviderRow {
                    slug,
                    kind,
                    display_name,
                    enabled,
                    allow_sign_up,
                    ..
                } = provider;
                println!(
                    "{slug}\t{kind}\t{display_name}\tenabled={enabled}\tsign_up={allow_sign_up}"
                );
            }
        }
        ProviderCommand::Disable { slug } => {
            let mut row = providers::by_slug(&mut tx, &slug)
                .await?
                .ok_or(Error::NotFound("provider"))?;
            row.enabled = false;
            providers::upsert(&mut tx, &row).await?;
        }
    }
    tx.commit().await?;
    pool.close().await;
    Ok(())
}

/// Runs a secret command.
pub(super) fn secret(command: SecretCommand) {
    let SecretCommand::Keygen = command;
    let (_, key) = Sealer::generate();
    println!("{key}");
}

/// Makes a provider that is on.
fn row(
    slug: String,
    kind: &str,
    name: String,
    settings: serde_json::Value,
    client_secret: Option<Vec<u8>>,
    allow_sign_up: bool,
) -> ProviderRow {
    ProviderRow {
        id: ProviderId::generate(),
        slug,
        kind: kind.to_owned(),
        display_name: name,
        settings,
        client_secret,
        allow_sign_up,
        enabled: true,
    }
}

/// Writes settings in JSON.
fn json(settings: &impl serde::Serialize) -> Result<serde_json::Value, Error> {
    serde_json::to_value(settings).map_err(|error| Error::Internal(error.to_string()))
}

/// Encrypts a client secret with the key of the settings.
fn seal(config: &Config, secret: &str) -> Result<Vec<u8>, Error> {
    let sealer = sealer(config)?.ok_or(Error::Config {
        name: "SCYLLA_SECRET_KEY",
        reason: "is required to store a client secret".to_owned(),
    })?;
    Ok(sealer.seal(secret))
}

/// Reads the first line of stdin, with no line end.
pub(super) fn stdin_line() -> Result<String, Error> {
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim_end_matches(['\r', '\n']).to_owned())
}
