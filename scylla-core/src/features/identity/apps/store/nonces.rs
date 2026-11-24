//! The SQL of the `assertion_nonces` table: each nonce of an assertion works one time.

use scylla_extension::Tx;
use time::OffsetDateTime;

/// Records a nonce. Returns `false` if the nonce is already known: a replay.
pub(crate) async fn use_nonce(
    tx: &mut Tx<'_>,
    nonce: &str,
    expires_at: OffsetDateTime,
) -> Result<bool, sqlx::Error> {
    // Forget the nonces of the expired assertions.
    sqlx::query!("DELETE FROM assertion_nonces WHERE expires_at < now()")
        .execute(&mut **tx)
        .await?;

    // Add the nonce. A known nonce adds no row.
    let result = sqlx::query!(
        "INSERT INTO assertion_nonces (nonce, expires_at) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        nonce,
        expires_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}
