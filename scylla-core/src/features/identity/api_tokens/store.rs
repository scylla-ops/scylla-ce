//! The SQL of the `api_tokens` table.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::api_tokens::{ApiTokenId, ApiTokenInfo, TokenName};
use scylla_extension::Tx;
use time::OffsetDateTime;

/// A new API token, with the hash of its secret.
pub(super) struct NewToken<'a> {
    /// The actor of the token.
    pub(super) actor: ActorId,
    /// The name.
    pub(super) name: &'a TokenName,
    /// The SHA-256 hash of the secret.
    pub(super) secret_hash: &'a [u8],
    /// The prefix and the last characters of the secret.
    pub(super) hint: &'a str,
    /// The actor that creates the token.
    pub(super) created_by: ActorId,
    /// The end of its validity.
    pub(super) expires_at: OffsetDateTime,
}

/// Inserts an API token, and returns it.
pub(super) async fn insert(
    tx: &mut Tx<'_>,
    new: NewToken<'_>,
) -> Result<ApiTokenInfo, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO api_tokens (id, actor_id, name, secret_hash, hint, created_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, actor_id, name, hint, created_by, created_at, expires_at, last_used_at,
                   revoked_at",
        ApiTokenId::generate().to_db(),
        new.actor.to_db(),
        new.name.as_str(),
        new.secret_hash,
        new.hint,
        new.created_by.to_db(),
        new.expires_at,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.into_info())
}

/// Returns the API tokens of an actor, the newest first.
pub(super) async fn list(
    tx: &mut Tx<'_>,
    actor: ActorId,
) -> Result<Vec<ApiTokenInfo>, sqlx::Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, actor_id, name, hint, created_by, created_at, expires_at, last_used_at,
                revoked_at
         FROM api_tokens WHERE actor_id = $1 ORDER BY created_at DESC, id DESC",
        actor.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_info).collect())
}

/// Revokes an API token of an actor. A revoked token keeps its first revocation time. Returns
/// `false` if the actor has no token with this id.
pub(super) async fn revoke(
    tx: &mut Tx<'_>,
    actor: ActorId,
    id: ApiTokenId,
) -> Result<bool, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE api_tokens SET revoked_at = coalesce(revoked_at, now())
         WHERE id = $1 AND actor_id = $2",
        id.to_db(),
        actor.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(done.rows_affected() == 1)
}

/// Finds the active token of a secret hash, sets its last use to now, and returns it with its
/// actor. A revoked or expired token gives `None`.
pub(super) async fn take(
    tx: &mut Tx<'_>,
    secret_hash: &[u8],
) -> Result<Option<(ApiTokenId, ActorId)>, sqlx::Error> {
    let row = sqlx::query!(
        "UPDATE api_tokens SET last_used_at = now()
         WHERE secret_hash = $1 AND revoked_at IS NULL AND expires_at > now()
         RETURNING id, actor_id",
        secret_hash,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| (ApiTokenId::from_db(row.id), ActorId::from_db(row.actor_id))))
}

/// Deletes each API token of an actor.
pub(super) async fn delete_all(tx: &mut Tx<'_>, actor: ActorId) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM api_tokens WHERE actor_id = $1", actor.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// A row of the `api_tokens` table, with no hash.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The actor.
    actor_id: uuid::Uuid,
    /// The name.
    name: String,
    /// The hint.
    hint: String,
    /// The creator.
    created_by: uuid::Uuid,
    /// The creation time.
    created_at: OffsetDateTime,
    /// The expiry.
    expires_at: OffsetDateTime,
    /// The last use.
    last_used_at: Option<OffsetDateTime>,
    /// The revocation time.
    revoked_at: Option<OffsetDateTime>,
}

impl Row {
    /// Makes the token of the row.
    fn into_info(self) -> ApiTokenInfo {
        ApiTokenInfo {
            id: ApiTokenId::from_db(self.id),
            actor_id: ActorId::from_db(self.actor_id),
            name: self.name,
            hint: self.hint,
            created_by: ActorId::from_db(self.created_by),
            created_at: self.created_at,
            expires_at: self.expires_at,
            last_used_at: self.last_used_at,
            revoked_at: self.revoked_at,
        }
    }
}
