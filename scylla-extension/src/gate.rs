//! Access control: each request proves its actor with a token, then passes permission checks.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use scylla_domain::actors::Actor;
use scylla_domain::authz::{Check, Object, Relationship};
use time::OffsetDateTime;
use tonic::metadata::MetadataMap;
use tonic::Request;

/// The gRPC method of a request, for example `scylla.projects.v1.ProjectService/CreateProject`.
/// The server adds it to each request. A token can restrict the operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operation(String);

impl Operation {
    /// Reads the operation from the path of a request: `/<service>/<method>`.
    pub fn from_path(path: &str) -> Self {
        Self(path.trim_start_matches('/').to_owned())
    }

    /// Returns the operation as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The actor of a request, proven by a token, and the operation of the request.
#[derive(Clone)]
pub struct Principal {
    /// The actor.
    pub actor: Actor,
    /// The operation of the request.
    operation: Operation,
    /// The verified token, opaque for the plugins.
    token: Arc<dyn Any + Send + Sync>,
}

impl Principal {
    /// Makes a principal. Only a gate makes one.
    pub fn new(actor: Actor, operation: Operation, token: Arc<dyn Any + Send + Sync>) -> Self {
        Self {
            actor,
            operation,
            token,
        }
    }

    /// Returns the operation of the request.
    pub fn operation(&self) -> &Operation {
        &self.operation
    }

    /// Returns the verified token, for the gate that made it.
    pub fn token(&self) -> &(dyn Any + Send + Sync) {
        self.token.as_ref()
    }
}

/// A short-lived token for an actor.
#[derive(Clone, Debug)]
pub struct IssuedToken {
    /// The token text.
    pub value: String,
    /// The end of its validity.
    pub expires_at: OffsetDateTime,
}

/// Why a request does not pass the gate.
#[derive(Debug, thiserror::Error)]
pub enum AccessError {
    /// No valid token: `UNAUTHENTICATED`.
    #[error("{0}")]
    Unauthenticated(String),
    /// The token or the permission system refuses: `PERMISSION_DENIED`.
    #[error("{0}")]
    Denied(String),
    /// The permission system does not answer: `UNAVAILABLE`.
    #[error("{0}")]
    Unavailable(String),
}

/// The gate of each request: tokens and permissions.
#[async_trait]
pub trait Gate: Send + Sync + 'static {
    /// Verifies the token of a request for an operation, and returns its principal.
    async fn authenticate(
        &self,
        metadata: &MetadataMap,
        operation: &Operation,
    ) -> Result<Principal, AccessError>;

    /// Passes if the token allows the operation on the object, and the actor has the permission.
    async fn authorize(&self, principal: &Principal, check: Check) -> Result<(), AccessError>;

    /// Makes a short-lived token for an actor.
    fn issue(&self, actor: &Actor) -> Result<IssuedToken, AccessError>;

    /// Writes relationships, and returns a token of the permission system for later checks.
    async fn write(&self, relationships: &[Relationship]) -> Result<String, AccessError>;

    /// Deletes relationships.
    async fn delete(&self, relationships: &[Relationship]) -> Result<(), AccessError>;

    /// Deletes all relationships of an object.
    async fn delete_object(&self, object: &Object) -> Result<(), AccessError>;
}

impl dyn Gate {
    /// Authenticates the caller of a gRPC request. The operation is the method of the request.
    pub async fn caller<T: Sync>(&self, request: &Request<T>) -> Result<Principal, AccessError> {
        let operation = request
            .extensions()
            .get::<Operation>()
            .ok_or_else(|| AccessError::Denied("the request has no operation".to_owned()))?;
        self.authenticate(request.metadata(), operation).await
    }
}
