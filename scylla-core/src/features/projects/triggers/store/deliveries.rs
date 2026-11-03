//! The SQL of the `trigger_deliveries` table: the keys of the deliveries of a webhook, so one
//! delivery fires one time.

use scylla_domain::projects::triggers::TriggerId;
use scylla_extension::Tx;
use time::OffsetDateTime;

/// Records the key of a delivery. Returns `false` if the trigger already has this key.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    trigger: TriggerId,
    key: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "INSERT INTO trigger_deliveries (trigger_id, key) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        trigger.to_db(),
        key,
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Forgets the key of a delivery whose fire failed, so a retry fires.
pub(crate) async fn delete(
    tx: &mut Tx<'_>,
    trigger: TriggerId,
    key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "DELETE FROM trigger_deliveries WHERE trigger_id = $1 AND key = $2",
        trigger.to_db(),
        key,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the keys older than `before`, and returns their count.
pub(crate) async fn purge(tx: &mut Tx<'_>, before: OffsetDateTime) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM trigger_deliveries WHERE created_at < $1",
        before
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected())
}
