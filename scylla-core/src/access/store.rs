//! The SQL of the token revocations.

use scylla_extension::Tx;

/// Returns `true` if one of the revocation ids of a token is revoked.
pub(super) async fn is_revoked(tx: &mut Tx<'_>, ids: &[Vec<u8>]) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT EXISTS (SELECT 1 FROM revoked_tokens WHERE revocation_id = ANY($1)) AS "revoked!""#,
        ids,
    )
    .fetch_one(&mut **tx)
    .await
}

/// Revokes a revocation id. A token that has it never passes again.
pub async fn revoke(tx: &mut Tx<'_>, id: &[u8]) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO revoked_tokens (revocation_id) VALUES ($1) ON CONFLICT DO NOTHING",
        id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
