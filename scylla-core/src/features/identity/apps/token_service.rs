//! The logic of the API tokens of the Apps: one transaction for each call.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::api_tokens::{
    ApiTokenId, ApiTokenInfo, TokenExpiry, TokenLimits, TokenName, APP_TOKEN_PREFIX,
};
use scylla_domain::identity::apps::{App, AppId, KIND};
use scylla_extension::{AccessError, Rejection};
use sqlx::PgPool;

use super::store::apps;
use crate::features::identity::api_tokens;
use crate::Error;

/// Creates an API token of an App, and returns it with its secret.
pub(crate) async fn create(
    pool: &PgPool,
    id: AppId,
    name: &TokenName,
    expiry: TokenExpiry,
    limits: &TokenLimits,
    created_by: ActorId,
) -> Result<(ApiTokenInfo, String), Error> {
    let mut tx = pool.begin().await?;
    let actor = App::actor(id);
    let prefix = APP_TOKEN_PREFIX;
    let created = api_tokens::create(&mut tx, actor, prefix, name, expiry, limits, created_by);
    let (token, secret) = created.await?;
    tx.commit().await?;
    tracing::info!(event = "app.token_created", app = %id, token = %token.id);
    Ok((token, secret))
}

/// Returns the API tokens of an App, the newest first.
pub(crate) async fn list(pool: &PgPool, id: AppId) -> Result<Vec<ApiTokenInfo>, Error> {
    let mut tx = pool.begin().await?;
    let tokens = api_tokens::list(&mut tx, App::actor(id)).await?;
    tx.commit().await?;
    Ok(tokens)
}

/// Revokes an API token of an App. A token of another App gives `NotFound("token")`.
pub(crate) async fn revoke(pool: &PgPool, id: AppId, token: ApiTokenId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !api_tokens::revoke(&mut tx, App::actor(id), token).await? {
        return Err(Error::NotFound("token"));
    }
    tx.commit().await?;
    tracing::info!(event = "app.token_revoked", app = %id, %token);
    Ok(())
}

/// Checks the secret of an API token of an App, records its use and the last sight of the App,
/// and returns the App, the token and its limits. A disabled App gets `APP_DISABLED`.
pub(crate) async fn exchange(
    pool: &PgPool,
    secret: &str,
) -> Result<(Actor, ApiTokenId, TokenLimits), Error> {
    let mut tx = pool.begin().await?;
    let found = api_tokens::take(&mut tx, APP_TOKEN_PREFIX, secret).await?;
    let taken = found.ok_or_else(refused)?;
    let id = App::of_actor(taken.actor);
    let app = apps::find(&mut tx, id).await?.ok_or_else(refused)?;
    if app.disabled_at.is_some() {
        let message = "the app is disabled";
        return Err(Error::Rejected(Rejection::new("APP_DISABLED", message)));
    }
    apps::touch(&mut tx, id).await?;
    tx.commit().await?;
    let app = Actor {
        id: taken.actor,
        kind: KIND,
    };
    Ok((app, taken.id, taken.limits))
}

/// The error of a secret that does not pass. It gives no detail to the client.
fn refused() -> Error {
    AccessError::Unauthenticated("invalid api token".to_owned()).into()
}
