//! The command `import`: the one-off import of the database of the old product.

use std::fmt;

use clap::Args;
use scylla_extension::Edition;
use sqlx::postgres::PgPoolOptions;

use crate::access::Access;
use crate::api;
use crate::features::platform::import;
use crate::infra::db;
use crate::infra::secret::Sealer;
use crate::{Config, Error};

/// The settings of the import. `Debug` hides the URL and the key.
#[derive(Args, Clone)]
#[group(id = "import")]
pub struct ImportArgs {
    /// The URL of the database of the old product.
    #[arg(long, env = "SCYLLA_IMPORT_FROM", hide_env_values = true)]
    pub from: String,
    /// The key in hex of the secrets of the old product (`[secrets] master_key`). With no key,
    /// the import leaves the secrets.
    #[arg(long, env = "SCYLLA_OLD_SECRET_KEY", hide_env_values = true)]
    pub old_secret_key: Option<String>,
}

impl fmt::Debug for ImportArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportArgs")
            .field("from", &"***")
            .field(
                "old_secret_key",
                &self.old_secret_key.as_ref().map(|_| "***"),
            )
            .finish()
    }
}

/// Imports the old database into the database of the settings, which must have no user, then
/// prints the report.
pub(super) async fn import(
    config: &Config,
    args: ImportArgs,
    edition: &Edition,
) -> Result<(), Error> {
    // Connect to the two databases, and check the migrations of this base.
    let pool = db::connect(config).await?;
    db::check_migrated(&pool, edition.migrators()).await?;
    let old = PgPoolOptions::new()
        .max_connections(2)
        .connect(&args.from)
        .await?;

    // Read the keys, then import.
    let access = Access::from_config(config, pool.clone(), edition.authz_model().clone())?;
    let sealer = api::sealer(config)?;
    let old_sealer = args
        .old_secret_key
        .as_deref()
        .map(Sealer::from_hex)
        .transpose()
        .map_err(|reason| Error::Config {
            name: "SCYLLA_OLD_SECRET_KEY",
            reason,
        })?;
    let report =
        import::import(&old, &pool, &access, sealer.as_deref(), old_sealer.as_ref()).await?;
    print!("{report}");
    Ok(())
}
