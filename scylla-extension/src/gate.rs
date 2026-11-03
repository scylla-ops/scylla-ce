//! Access control: each request proves its actor with a token, then passes permission checks.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use scylla_domain::authz::{Check, Object, Permission, Relationship, Resource};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::SessionId;
use scylla_domain::Id;
use time::OffsetDateTime;
use tonic::metadata::MetadataMap;
use tonic::Request;

use crate::Operation;

/// The actor of a request, proven by a token, and the operation of the request.
#[derive(Clone)]
pub struct Principal {
    /// The actor.
    pub actor: Actor,
    /// The operation of the request.
    operation: Operation,
    /// The verified token, opaque for the plugins.
    token: Arc<dyn Any + Send + Sync>,
    /// The session of the token, for a user that signed in.
    session: Option<SessionId>,
}

impl Principal {
    /// Makes a principal. Only a gate makes one.
    pub fn new(
        actor: Actor,
        operation: Operation,
        token: Arc<dyn Any + Send + Sync>,
        session: Option<SessionId>,
    ) -> Self {
        Self {
            actor,
            operation,
            token,
            session,
        }
    }

    /// Returns the session of the token, if the actor signed in as a user.
    pub fn session(&self) -> Option<SessionId> {
        self.session
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

impl AccessError {
    /// Makes the refusal of a check. A route gives the same refusal for an object that does not
    /// exist, so no caller learns which ids exist.
    pub fn denied(check: &Check) -> Self {
        Self::Denied(format!("{check} is denied"))
    }
}

/// The gate of each request: tokens and permissions. Each route checks its own permission: the
/// server refuses the success of a route that verified a token but passed no such check.
#[async_trait]
pub trait Gate: Send + Sync + 'static {
    /// Verifies the token of a request for an operation, records it on the operation, and
    /// returns the principal.
    async fn authenticate(
        &self,
        metadata: &MetadataMap,
        operation: &Operation,
    ) -> Result<Principal, AccessError>;

    /// Passes if the check is of the permission of the route, the token allows the operation on
    /// the object, and the actor has the permission. Records the passed check on the operation.
    async fn authorize(&self, principal: &Principal, check: Check) -> Result<(), AccessError>;

    /// Makes a short-lived token for an actor, in a session for a user that signed in. A token
    /// of a revoked session stops at once.
    fn issue(&self, actor: &Actor, session: Option<SessionId>) -> Result<IssuedToken, AccessError>;

    /// Writes relationships, and returns a token of the permission system for later checks.
    async fn write(&self, relationships: &[Relationship]) -> Result<String, AccessError>;

    /// Deletes relationships.
    async fn delete(&self, relationships: &[Relationship]) -> Result<(), AccessError>;

    /// Deletes all relationships of an object.
    async fn delete_object(&self, object: &Object) -> Result<(), AccessError>;

    /// Deletes all relationships of a subject: each role of an actor, on each object.
    async fn delete_subject(&self, subject: &Object) -> Result<(), AccessError>;

    /// Returns the actors that have a relation on an object: each actor with a role on it.
    async fn subjects(&self, object: &Object) -> Result<Vec<ActorId>, AccessError>;

    /// Returns the ids of the objects of a definition on which the actor of the principal has a
    /// role that gives a permission. The workspaces that the actor owns are not in the answer:
    /// the route adds them from Postgres.
    async fn lookup(
        &self,
        principal: &Principal,
        definition: &'static str,
        permission: &'static str,
    ) -> Result<Vec<String>, AccessError>;
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

    /// Returns the objects of `R` on which the caller has a grant of the permission, from a
    /// reverse lookup in SpiceDB: `gate.granted(&caller, Project::LIST_PROJECTS)`.
    pub async fn granted<R: Resource>(
        &self,
        principal: &Principal,
        permission: Permission<R>,
    ) -> Result<Vec<Id<R>>, AccessError> {
        let ids = self.lookup(principal, R::NAME, permission.name()).await?;
        Ok(ids.iter().filter_map(|id| Id::parse(id).ok()).collect())
    }
}
