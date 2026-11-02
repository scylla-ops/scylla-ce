//! The SQL of the gate: the token revocations and the owner chains.

use scylla_domain::actors::ActorId;
use scylla_extension::{AccessError, Tx};

/// The most actors in an owner chain. A longer chain stops there.
const MAX_CHAIN: i32 = 8;

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

/// Returns an actor and the actors above it, nearest first. An unknown actor gives none.
pub(super) async fn owner_chain(
    tx: &mut Tx<'_>,
    actor: ActorId,
) -> Result<Vec<ActorId>, sqlx::Error> {
    let ids = sqlx::query_scalar!(
        r#"WITH RECURSIVE chain (id, owner_id, depth) AS (
               SELECT id, owner_id, 1 FROM actors WHERE id = $1
               UNION ALL
               SELECT a.id, a.owner_id, c.depth + 1
               FROM actors a JOIN chain c ON a.id = c.owner_id
               WHERE c.depth < $2
           )
           SELECT id AS "id!" FROM chain ORDER BY depth"#,
        actor.to_db(),
        MAX_CHAIN,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(ids.into_iter().map(ActorId::from_db).collect())
}

/// Logs a database failure of the gate, and hides its details from the client.
pub(super) fn failed(error: sqlx::Error) -> AccessError {
    tracing::error!(%error, "the access store failed");
    AccessError::Unavailable("access check unavailable".to_owned())
}
