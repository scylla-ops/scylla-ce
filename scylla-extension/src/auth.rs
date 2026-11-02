//! Authentication: find the actor that sends a request.

use std::sync::Arc;

use async_trait::async_trait;
use scylla_domain::actors::Actor;
use sqlx::PgPool;
use tonic::metadata::MetadataMap;

/// Why a request has no actor.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// No authenticator found a credential. The client gets `UNAUTHENTICATED`.
    #[error("no credential")]
    Missing,
    /// A credential is not valid. The client gets `UNAUTHENTICATED`.
    #[error("{0}")]
    Invalid(String),
    /// The authenticator failed on the database. The client gets `INTERNAL`.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Finds the actor of a request from one kind of credential, for example a token.
#[async_trait]
pub trait Authenticator: Send + Sync + 'static {
    /// Returns the actor, or `None` if the request has no credential of this kind.
    async fn authenticate(
        &self,
        pool: &PgPool,
        metadata: &MetadataMap,
    ) -> Result<Option<Actor>, AuthError>;
}

/// The authenticators of the server, tried in registration order.
#[derive(Clone, Default)]
pub struct Authenticators(Vec<Arc<dyn Authenticator>>);

impl Authenticators {
    /// Makes the chain from a list of authenticators.
    pub fn new(authenticators: Vec<Arc<dyn Authenticator>>) -> Self {
        Self(authenticators)
    }

    /// Returns the actor of the first authenticator that finds a credential.
    pub async fn authenticate(
        &self,
        pool: &PgPool,
        metadata: &MetadataMap,
    ) -> Result<Actor, AuthError> {
        for authenticator in &self.0 {
            if let Some(actor) = authenticator.authenticate(pool, metadata).await? {
                return Ok(actor);
            }
        }
        Err(AuthError::Missing)
    }

    /// Returns `true` if the chain has no authenticator.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
