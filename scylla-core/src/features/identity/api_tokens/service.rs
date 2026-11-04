//! The logic of the API tokens, in the transaction of the caller: the feature of the actor
//! checks the actor and commits.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::api_tokens::{
    self, ApiTokenId, ApiTokenInfo, TokenExpiry, TokenLimits, TokenName,
};
use scylla_extension::{AccessError, Principal, Tx};
use time::OffsetDateTime;

use super::store::{self, NewToken, Taken};
use crate::infra::secret;
use crate::Error;

/// Creates an API token of an actor with a new secret that starts with `prefix`, and returns
/// the token and its secret. Only the hash of the secret goes to the database.
pub(crate) async fn create(
    tx: &mut Tx<'_>,
    actor: ActorId,
    prefix: &str,
    name: &TokenName,
    expiry: TokenExpiry,
    limits: &TokenLimits,
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
        limits,
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
/// it with its actor and its limits. Another prefix, or an unknown, revoked or expired secret,
/// gives `None`.
pub(crate) async fn take(
    tx: &mut Tx<'_>,
    prefix: &str,
    secret: &str,
) -> Result<Option<Taken>, sqlx::Error> {
    if !secret.starts_with(prefix) {
        return Ok(None);
    }
    store::take(tx, &secret::hash(secret)).await
}

/// Deletes each API token of an actor.
pub(crate) async fn delete_all(tx: &mut Tx<'_>, actor: ActorId) -> Result<(), sqlx::Error> {
    store::delete_all(tx, actor).await
}

/// Returns the number of tokens whose secret starts with `prefix`, not revoked and not expired.
pub(crate) async fn count_active(tx: &mut Tx<'_>, prefix: &str) -> Result<u32, sqlx::Error> {
    store::count_active(tx, prefix).await
}

/// Refuses a caller whose token came from an API token: an API token cannot make another one.
pub(crate) fn check_maker(caller: &Principal) -> Result<(), Error> {
    match caller.api_token() {
        Some(_) => {
            let reason = "a token of an API token cannot create an API token".to_owned();
            Err(AccessError::Denied(reason).into())
        }
        None => Ok(()),
    }
}
