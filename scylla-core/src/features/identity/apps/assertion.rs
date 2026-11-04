//! The check of a public key and of an assertion that an App signed with its private key.

use ed25519_dalek::{Signature, VerifyingKey};
use prost::Message;
use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::apps::{App, AppId, AppKeyId, KIND};
use scylla_extension::{AccessError, Rejection};
use scylla_proto::apps::v1::AppAssertion;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

use super::store::{apps, keys, nonces};
use crate::api::parse;
use crate::Error;

/// The audience that each assertion must name.
pub const AUDIENCE: &str = "scylla";

/// The longest life of an assertion.
const MAX_LIFETIME: Duration = Duration::minutes(2);

/// The shortest nonce.
const MIN_NONCE_LEN: usize = 16;

/// Refuses a key that is not an Ed25519 public key.
pub(crate) fn check_public_key(public_key: &[u8]) -> Result<(), Error> {
    let invalid = || Error::invalid("public_key", "must be an Ed25519 public key");
    let bytes: [u8; 32] = public_key.try_into().map_err(|_| invalid())?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| invalid())?;
    Ok(())
}

/// Checks an assertion that an App signed, and returns the App and the key that signed. A
/// disabled App gets `APP_DISABLED`.
pub async fn verify(
    pool: &PgPool,
    assertion: &[u8],
    signature: &[u8],
) -> Result<(Actor, AppKeyId), Error> {
    // Read the claims of the assertion.
    let claims = AppAssertion::decode(assertion).map_err(|_| refused())?;
    let app_id: AppId = parse::required(claims.app_id, "app_id").map_err(|_| refused())?;
    let key_id: AppKeyId = parse::required(claims.key_id, "key_id").map_err(|_| refused())?;
    let expires_at = claims
        .expires_at
        .and_then(|at| OffsetDateTime::from_unix_timestamp(at.seconds).ok())
        .ok_or_else(refused)?;

    // Refuse a wrong audience, an expiry out of range, or a short nonce.
    let now = OffsetDateTime::now_utc();
    if claims.audience != AUDIENCE
        || expires_at <= now
        || expires_at > now + MAX_LIFETIME
        || claims.nonce.len() < MIN_NONCE_LEN
    {
        return Err(refused());
    }

    // Verify the signature with the active key of this App.
    let mut tx = pool.begin().await?;
    let key = keys::find(&mut tx, key_id)
        .await?
        .filter(|key| key.app_id == app_id && !key.revoked)
        .ok_or_else(refused)?;
    let public_key: [u8; 32] = key.public_key.try_into().map_err(|_| refused())?;
    let public_key = VerifyingKey::from_bytes(&public_key).map_err(|_| refused())?;
    let signature = Signature::from_slice(signature).map_err(|_| refused())?;
    public_key
        .verify_strict(assertion, &signature)
        .map_err(|_| refused())?;

    // Refuse a disabled App.
    let disabled = apps::find(&mut tx, app_id)
        .await?
        .is_some_and(|app| app.disabled_at.is_some());
    if disabled {
        let message = "the app is disabled";
        return Err(Error::Rejected(Rejection::new("APP_DISABLED", message)));
    }

    // Use the nonce one time: a replay of the assertion fails here.
    if !nonces::use_nonce(&mut tx, &claims.nonce, expires_at).await? {
        return Err(refused());
    }
    keys::touch(&mut tx, key_id).await?;
    apps::touch(&mut tx, app_id).await?;
    tx.commit().await?;
    let app = Actor {
        id: App::actor(app_id),
        kind: KIND,
    };
    Ok((app, key_id))
}

/// The error of an assertion that does not pass. It gives no detail to the client.
fn refused() -> Error {
    AccessError::Unauthenticated("invalid assertion".to_owned()).into()
}
