//! The authenticator of the API keys.

use scylla_domain::actors::Actor;
use scylla_extension::{async_trait, AuthError, Authenticator};
use sqlx::PgPool;
use tonic::metadata::MetadataMap;

use super::service::{hash, HEADER};
use super::store;

/// Finds the actor of the key in `authorization: Bearer <key>`.
pub(super) struct KeyAuthenticator;

#[async_trait]
impl Authenticator for KeyAuthenticator {
    async fn authenticate(
        &self,
        pool: &PgPool,
        metadata: &MetadataMap,
    ) -> Result<Option<Actor>, AuthError> {
        let Some(value) = metadata.get(HEADER) else {
            return Ok(None);
        };
        let secret = value
            .to_str()
            .ok()
            .and_then(|text| text.strip_prefix("Bearer "))
            .ok_or_else(|| AuthError::Invalid(format!("{HEADER} must be Bearer <key>")))?;
        let mut tx = pool.begin().await?;
        let actor = store::find_actor(&mut tx, &hash(secret)).await?;
        tx.commit().await?;
        actor
            .map(Some)
            .ok_or_else(|| AuthError::Invalid("unknown or revoked key".to_owned()))
    }
}
