//! The logic of the CI identities of the Apps: one transaction for each call, and the exchange
//! of the ID token of a CI job with the keys of its issuer.

use std::collections::HashMap;

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::api_tokens::TokenLimits;
use scylla_domain::identity::apps::{App, AppId, KIND};
use scylla_domain::identity::ci_identities::{
    CiClaims, CiIdentityId, CiIdentityInfo, CiProvider, NewCiIdentity,
};
use scylla_extension::{AccessError, Rejection};
use serde_json::Value;
use sqlx::PgPool;

use super::store::{apps, ci};
use crate::features::identity::providers::Providers;
use crate::Error;

/// Adds a CI identity to an App, and returns it.
pub(super) async fn add(
    pool: &PgPool,
    id: AppId,
    new: &NewCiIdentity,
    created_by: ActorId,
) -> Result<CiIdentityInfo, Error> {
    let mut tx = pool.begin().await?;
    let identity = ci::insert(&mut tx, id, new, created_by).await?;
    tx.commit().await?;
    tracing::info!(event = "app.ci_identity_added", app = %id, identity = %identity.id);
    Ok(identity)
}

/// Returns the CI identities of an App, the oldest first.
pub(super) async fn list(pool: &PgPool, id: AppId) -> Result<Vec<CiIdentityInfo>, Error> {
    let mut tx = pool.begin().await?;
    let identities = ci::list(&mut tx, id).await?;
    tx.commit().await?;
    Ok(identities)
}

/// Removes a CI identity of an App. One of another App gives `NotFound("ci_identity")`.
pub(super) async fn remove(pool: &PgPool, id: AppId, identity: CiIdentityId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !ci::remove(&mut tx, id, identity).await? {
        return Err(Error::NotFound("ci_identity"));
    }
    tx.commit().await?;
    tracing::info!(event = "app.ci_identity_removed", app = %id, %identity);
    Ok(())
}

/// Checks the ID token of a CI job with the issuer of each CI identity of an App, and returns the
/// App and the limits of the first CI identity whose rules fit the claims. A disabled App gets
/// `APP_DISABLED`.
pub(crate) async fn exchange(
    pool: &PgPool,
    providers: &Providers,
    audience: &str,
    id: AppId,
    id_token: &str,
) -> Result<(Actor, TokenLimits), Error> {
    // Read the CI identities, then check the token with each issuer, one time each.
    let identities = list(pool, id).await?;
    let mut checked: HashMap<&str, Option<HashMap<String, Value>>> = HashMap::new();
    let mut found = None;
    for identity in &identities {
        let issuer = identity.issuer.as_str();
        if !checked.contains_key(issuer) {
            let claims = providers.verify_ci(issuer, audience, id_token).await?;
            checked.insert(issuer, claims);
        }
        let claims = checked.get(issuer).and_then(Option::as_ref);
        if claims.is_some_and(|claims| identity.matches(&read(identity.provider, claims))) {
            found = Some(identity);
            break;
        }
    }
    let identity = found.ok_or_else(refused)?;

    // Refuse a disabled App, and record the use.
    let mut tx = pool.begin().await?;
    let app = apps::find(&mut tx, id).await?.ok_or_else(refused)?;
    if app.disabled_at.is_some() {
        let message = "the app is disabled";
        return Err(Error::Rejected(Rejection::new("APP_DISABLED", message)));
    }
    if !ci::touch(&mut tx, identity.id).await? {
        return Err(refused());
    }
    apps::touch(&mut tx, id).await?;
    tx.commit().await?;
    tracing::info!(event = "app.ci_signed_in", app = %id, identity = %identity.id);
    let actor = Actor {
        id: App::actor(id),
        kind: KIND,
    };
    Ok((actor, identity.limits.clone()))
}

/// Reads the claims of a CI provider in the claims of an ID token.
fn read(provider: CiProvider, claims: &HashMap<String, Value>) -> CiClaims {
    let text = |name: &str| claims.get(name).and_then(Value::as_str).map(str::to_owned);
    CiClaims {
        repository: text(provider.repository_claim()),
        git_ref: text("ref"),
        environment: text("environment"),
    }
}

/// The error of an ID token that does not pass. It gives no detail to the client.
fn refused() -> Error {
    AccessError::Unauthenticated("invalid CI token".to_owned()).into()
}
