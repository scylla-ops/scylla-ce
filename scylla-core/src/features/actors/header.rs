//! The dev authenticator: it trusts the actor id of a header. Never use it in production.

use scylla_domain::actors::{Actor, ActorId};
use scylla_extension::{async_trait, AuthError, Authenticator};
use sqlx::PgPool;
use tonic::metadata::MetadataMap;

use super::store;

/// The header that holds the id of the actor.
pub const ACTOR_HEADER: &str = "x-scylla-actor";

/// Takes the actor of `x-scylla-actor`, if the actor exists. Any client can claim any actor.
pub struct TrustedHeader;

#[async_trait]
impl Authenticator for TrustedHeader {
    async fn authenticate(
        &self,
        pool: &PgPool,
        metadata: &MetadataMap,
    ) -> Result<Option<Actor>, AuthError> {
        let Some(value) = metadata.get(ACTOR_HEADER) else {
            return Ok(None);
        };
        let id = value
            .to_str()
            .ok()
            .and_then(|text| ActorId::parse(text).ok())
            .ok_or_else(|| AuthError::Invalid(format!("{ACTOR_HEADER} must be an actor id")))?;
        let mut tx = pool.begin().await?;
        let actor = store::find(&mut tx, id).await?;
        tx.commit().await?;
        actor
            .map(Some)
            .ok_or_else(|| AuthError::Invalid("unknown actor".to_owned()))
    }
}
