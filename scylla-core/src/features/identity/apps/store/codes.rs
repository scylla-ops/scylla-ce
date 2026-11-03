//! The SQL of the `app_join_codes` table. The table keeps the hash of each code only.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::apps::AppId;
use scylla_extension::Tx;
use time::OffsetDateTime;

/// Stops the open codes of an App, then inserts the hash of a new code.
pub(crate) async fn replace(
    tx: &mut Tx<'_>,
    app: AppId,
    hash: &[u8],
    created_by: ActorId,
    expires_at: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    // Stop the open codes of the App.
    sqlx::query!(
        "UPDATE app_join_codes SET expires_at = now()
         WHERE app_id = $1 AND used_at IS NULL AND expires_at > now()",
        app.to_db(),
    )
    .execute(&mut **tx)
    .await?;

    // Insert the new code.
    sqlx::query!(
        "INSERT INTO app_join_codes (hash, app_id, created_by, expires_at) VALUES ($1, $2, $3, $4)",
        hash,
        app.to_db(),
        created_by.to_db(),
        expires_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Uses a code one time, and returns its App, or `None` if the code is unknown, used or
/// expired. The row lock makes two uses of one code give one App.
pub(crate) async fn take(tx: &mut Tx<'_>, hash: &[u8]) -> Result<Option<AppId>, sqlx::Error> {
    let app = sqlx::query_scalar!(
        "UPDATE app_join_codes SET used_at = now()
         WHERE hash = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING app_id",
        hash,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(app.map(AppId::from_db))
}
