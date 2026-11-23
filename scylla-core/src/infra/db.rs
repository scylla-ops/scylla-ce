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

#[cfg(test)]
mod tests {
    //! Unit tests of the migrations: the lock of the released files, and their line ends.

    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    use super::MIGRATOR;

    /// The released migrations: one line `<version> <SHA-384 in hex>` for each file.
    const LOCK: &str = include_str!("../../migrations.lock");

    /// Returns the version and the checksum of each line of the lock.
    fn locked() -> BTreeMap<i64, &'static str> {
        LOCK.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let (version, checksum) = line
                    .split_once(' ')
                    .expect("a line has a version and a checksum");
                (
                    version.parse().expect("a version is a number"),
                    checksum.trim(),
                )
            })
            .collect()
    }

    /// Returns the version and the checksum in hex of each migration, as sqlx keeps them.
    fn current() -> BTreeMap<i64, String> {
        MIGRATOR
            .iter()
            .filter(|migration| !migration.migration_type.is_down_migration())
            .map(|migration| {
                let checksum = migration.checksum.iter().map(|byte| format!("{byte:02x}"));
                (migration.version, checksum.collect())
            })
            .collect()
    }

    /// A released migration keeps its file, and a new migration comes after the last released one.
    #[test]
    fn a_released_migration_never_changes() {
        // given
        let locked = locked();
        let last = locked.keys().max().copied().unwrap_or_default();

        // when
        let current = current();

        // then
        for (version, checksum) in &locked {
            assert_eq!(
                current.get(version).map(String::as_str),
                Some(*checksum),
                "the released migration {version} changed or is gone: restore its file, and put the change in a new migration (decision 39)"
            );
        }
        for version in current
            .keys()
            .filter(|version| !locked.contains_key(version))
        {
            assert!(
                *version > last,
                "the new migration {version} is not after the last released migration {last}: give it a greater version"
            );
        }
    }

    /// No migration has a CR: sqlx keeps the checksum of the bytes, so a CRLF copy stops the server.
    #[test]
    fn a_migration_has_no_carriage_return() {
        // given
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");

        // when
        let with_cr: Vec<String> = fs::read_dir(&folder)
            .expect("the migrations folder")
            .map(|entry| entry.expect("a migration file").path())
            .filter(|path| fs::read(path).expect("a migration file").contains(&b'\r'))
            .map(|path| path.display().to_string())
            .collect();

        // then
        assert!(
            with_cr.is_empty(),
            "these migrations have a CR, check them out with LF: {with_cr:#?}"
        );
    }
}
