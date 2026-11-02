//! The SQL of the `api_keys` table.

use scylla_domain::actors::{Actor, ActorId, ActorKind};
use scylla_domain::api_keys::ApiKeyId;
use scylla_extension::Tx;

use crate::Error;

/// Inserts the hash of a key. An unknown actor gives `NotFound("actor")`.
pub(super) async fn insert(
    tx: &mut Tx<'_>,
    id: ApiKeyId,
    actor_id: ActorId,
    hash: &[u8],
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO api_keys (id, actor_id, hash) VALUES ($1, $2, $3)",
        id.to_db(),
        actor_id.to_db(),
        hash,
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.is_foreign_key_violation() => Error::NotFound("actor"),
        error => error.into(),
    })?;
    Ok(())
}

/// Returns the actor of a key hash, or `None` if no key that is not revoked has this hash.
pub(super) async fn find_actor(tx: &mut Tx<'_>, hash: &[u8]) -> Result<Option<Actor>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT a.id, a.kind FROM api_keys k JOIN actors a ON a.id = k.actor_id
         WHERE k.hash = $1 AND k.revoked_at IS NULL",
        hash,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Actor {
        id: ActorId::from_db(row.id),
        kind: ActorKind::from_name(row.kind),
    }))
}

/// Marks a key as revoked, and keeps the first time. Returns `false` if the key is unknown.
pub(super) async fn revoke(tx: &mut Tx<'_>, id: ApiKeyId) -> Result<bool, Error> {
    let result = sqlx::query!(
        "UPDATE api_keys SET revoked_at = COALESCE(revoked_at, now()) WHERE id = $1",
        id.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}
