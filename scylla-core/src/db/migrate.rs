use sqlx::migrate::Migrator;
use sqlx::PgPool;

use crate::Error;

pub static MIGRATOR: Migrator = sqlx::migrate!();

pub async fn migrate(pool: &PgPool, edition: &[&Migrator]) -> Result<(), Error> {
    MIGRATOR.run(pool).await?;
    for migrator in edition {
        migrator.run(pool).await?;
    }
    Ok(())
}
