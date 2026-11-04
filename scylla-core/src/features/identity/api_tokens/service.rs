//! The logic of the API tokens, in the transaction of the caller: the feature of the actor
//! checks the actor and commits.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::api_tokens::{self, ApiTokenId, ApiTokenInfo, TokenExpiry, TokenName};
use scylla_extension::Tx;
use time::OffsetDateTime;

use super::store::{self, NewToken};
use crate::infra::secret;

/// Creates an API token of an actor with a new secret that starts with `prefix`, and returns
/// the token and its secret. Only the hash of the secret goes to the database.
pub(crate) async fn create(
    tx: &mut Tx<'_>,
    actor: ActorId,
    prefix: &str,
    name: &TokenName,
    expiry: TokenExpiry,
    created_by: ActorId,
) -> Result<(ApiTokenInfo, String), sqlx::Error> {
    let secret = format!("{prefix}{}", secret::token());
    let new = NewToken {
        actor,
        name,
        secret_hash: &secret::hash(&secret),
        hint: &api_tokens::hint(prefix, &secret),
        created_by,
        expires_at: OffsetDateTime::now_utc() + expiry.duration(),
    };
    let token = store::insert(tx, new).await?;
    Ok((token, secret))
}

/// Returns the API tokens of an actor, the newest first.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    actor: ActorId,
) -> Result<Vec<ApiTokenInfo>, sqlx::Error> {
    store::list(tx, actor).await
}

/// Revokes an API token of an actor. Returns `false` if the actor has no token with this id.
pub(crate) async fn revoke(
    tx: &mut Tx<'_>,
    actor: ActorId,
    id: ApiTokenId,
) -> Result<bool, sqlx::Error> {
    store::revoke(tx, actor, id).await
}

/// Finds the active token of a secret that starts with `prefix`, records its use, and returns
/// it with its actor. Another prefix, or an unknown, revoked or expired secret, gives `None`.
pub(crate) async fn take(
    tx: &mut Tx<'_>,
    prefix: &str,
    secret: &str,
) -> Result<Option<(ApiTokenId, ActorId)>, sqlx::Error> {
    if !secret.starts_with(prefix) {
        return Ok(None);
    }
    store::take(tx, &secret::hash(secret)).await
}

/// Deletes each API token of an actor.
pub(crate) async fn delete_all(tx: &mut Tx<'_>, actor: ActorId) -> Result<(), sqlx::Error> {
    store::delete_all(tx, actor).await
}
