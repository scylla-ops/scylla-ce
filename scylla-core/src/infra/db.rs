//! The database: the pool, the core migrations, and the schema check at start.

use std::collections::HashSet;
use std::str::FromStr;

use sqlx::migrate::Migrator;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};

use crate::{Config, Error};

/// The core migrations, in `scylla-core/migrations`, with the ledger `_sqlx_migrations_core`.
pub static MIGRATOR: Migrator = sqlx::migrate!();

/// The Postgres error code of a missing table.
const UNDEFINED_TABLE: &str = "42P01";

/// Opens the pool, with the pool limits and a `statement_timeout` on each connection.
pub async fn connect(config: &Config) -> Result<PgPool, Error> {
    let statement_timeout = config.db_statement_timeout.as_millis().to_string();
    let options = PgConnectOptions::from_str(&config.database_url)?
        .options([("statement_timeout", statement_timeout.as_str())]);
    let pool = PgPoolOptions::new()
        .max_connections(config.db_max_connections)
        .acquire_timeout(config.db_acquire_timeout)
        .connect_with(options)
        .await?;
    Ok(pool)
}

/// Applies each migrator, in order. sqlx locks each ledger, so two jobs do not conflict.
pub async fn migrate(pool: &PgPool, migrators: &[&Migrator]) -> Result<(), Error> {
    for migrator in migrators {
        migrator.run(pool).await?;
    }
    Ok(())
}

/// Fails if a migration is not applied: the server never migrates by itself.
pub async fn check_migrated(pool: &PgPool, migrators: &[&Migrator]) -> Result<(), Error> {
    for migrator in migrators {
        let applied = applied_versions(pool, &migrator.table_name).await?;
        let pending = migrator
            .iter()
            .filter(|migration| !migration.migration_type.is_down_migration())
            .filter(|migration| !applied.contains(&migration.version))
            .count();
        if pending > 0 {
            return Err(Error::PendingMigrations {
                table: migrator.table_name.to_string(),
                count: pending,
            });
        }
    }
    Ok(())
}

/// Returns the applied versions of a ledger. A missing ledger gives an empty set.
async fn applied_versions(pool: &PgPool, table: &str) -> Result<HashSet<i64>, Error> {
    let sql = AssertSqlSafe(format!("SELECT version FROM {table} WHERE success"));
    match sqlx::query_scalar::<_, i64>(sql).fetch_all(pool).await {
        Ok(versions) => Ok(versions.into_iter().collect()),
        Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some(UNDEFINED_TABLE) => {
            Ok(HashSet::new())
        }
        Err(error) => Err(error.into()),
    }
}
