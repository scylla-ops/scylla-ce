//! The logic of the personal tokens: one transaction for each call. The switch of the instance
//! stops their creation and their exchange.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::api_tokens::{
    ApiTokenId, ApiTokenInfo, TokenExpiry, TokenLimits, TokenName, PERSONAL_TOKEN_PREFIX,
};
use scylla_domain::identity::users::KIND;
use scylla_extension::{AccessError, Rejection};
use sqlx::PgPool;

use crate::features::identity::api_tokens;
use crate::features::platform::settings;
use crate::Error;

/// Creates a personal token of a user, and returns it with its secret.
pub(super) async fn create(
    pool: &PgPool,
    user: ActorId,
    name: &TokenName,
    expiry: TokenExpiry,
    limits: &TokenLimits,
) -> Result<(ApiTokenInfo, String), Error> {
    let mut tx = pool.begin().await?;
    if !settings::personal_tokens_enabled(&mut tx).await? {
        return Err(disabled());
    }
    let prefix = PERSONAL_TOKEN_PREFIX;
    let created = api_tokens::create(&mut tx, user, prefix, name, expiry, limits, user);
    let (token, secret) = created.await?;
    tx.commit().await?;
    tracing::info!(event = "user.token_created", %user, token = %token.id);
    Ok((token, secret))
}

/// Returns the personal tokens of a user, the newest first, and `true` when the instance allows
/// them.
pub(super) async fn list(pool: &PgPool, user: ActorId) -> Result<(Vec<ApiTokenInfo>, bool), Error> {
    let mut tx = pool.begin().await?;
    let tokens = api_tokens::list(&mut tx, user).await?;
    let enabled = settings::personal_tokens_enabled(&mut tx).await?;
    tx.commit().await?;
    Ok((tokens, enabled))
}

/// Revokes a personal token of a user. A token of another user gives `NotFound("token")`.
pub(super) async fn revoke(pool: &PgPool, user: ActorId, token: ApiTokenId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !api_tokens::revoke(&mut tx, user, token).await? {
        return Err(Error::NotFound("token"));
    }
    tx.commit().await?;
    tracing::info!(event = "user.token_revoked", %user, %token);
    Ok(())
}

/// Checks the secret of a personal token, records its use, and returns the user, the token and
/// its limits. An instance with the personal tokens off gives `PERSONAL_TOKENS_DISABLED`.
pub(crate) async fn exchange(
    pool: &PgPool,
    secret: &str,
) -> Result<(Actor, ApiTokenId, TokenLimits), Error> {
    let mut tx = pool.begin().await?;
    let found = api_tokens::take(&mut tx, PERSONAL_TOKEN_PREFIX, secret).await?;
    let taken = found.ok_or_else(refused)?;
    if !settings::personal_tokens_enabled(&mut tx).await? {
        return Err(disabled());
    }
    tx.commit().await?;
    let user = Actor {
        id: taken.actor,
        kind: KIND,
    };
    Ok((user, taken.id, taken.limits))
}

/// The refusal of an instance with the personal tokens off.
fn disabled() -> Error {
    let message = "the personal tokens are off on this server";
    Error::Rejected(Rejection::new("PERSONAL_TOKENS_DISABLED", message))
}

/// The error of a secret that does not pass. It gives no detail to the client.
fn refused() -> Error {
    AccessError::Unauthenticated("invalid api token".to_owned()).into()
}
