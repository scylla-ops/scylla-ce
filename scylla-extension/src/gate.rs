//! Access control: each request proves its actor with a token, then passes a permission check.

use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use scylla_domain::actors::Actor;
use time::OffsetDateTime;
use tonic::metadata::MetadataMap;

/// An object of the permission system, for example `project:<id>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    /// The type of the object, a definition of the SpiceDB schema.
    pub kind: &'static str,
    /// The id of the object.
    pub id: String,
}

impl Object {
    /// Makes an object reference.
    pub fn new(kind: &'static str, id: impl ToString) -> Self {
        Self {
            kind,
            id: id.to_string(),
        }
    }
}

/// A relation between two objects, for example `project:P#owner@actor:A`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    /// The object that has the relation.
    pub object: Object,
    /// The name of the relation.
    pub relation: &'static str,
    /// The subject of the relation.
    pub subject: Object,
}

/// What a request needs: an operation on an object, and a permission on that object.
#[derive(Clone, Copy, Debug)]
pub struct Check<'a> {
    /// The name of the operation, for example `projects.create`. A token can restrict it.
    pub operation: &'static str,
    /// The object of the operation.
    pub object: &'a Object,
    /// The permission that the actor needs on the object.
    pub permission: &'static str,
    /// A token of the permission system from a write: the check sees at least that write.
    pub at_least_as_fresh: Option<&'a str>,
}

/// The actor of a request, proven by a token.
#[derive(Clone)]
pub struct Principal {
    /// The actor.
    pub actor: Actor,
    /// The verified token, opaque for the plugins.
    token: Arc<dyn Any + Send + Sync>,
}

impl Principal {
    /// Makes a principal. Only a gate makes one.
    pub fn new(actor: Actor, token: Arc<dyn Any + Send + Sync>) -> Self {
        Self { actor, token }
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
    /// Verifies the token of a request and returns its actor.
    async fn authenticate(&self, metadata: &MetadataMap) -> Result<Principal, AccessError>;

    /// Passes if the token allows the operation and the actor has the permission.
    async fn authorize(&self, principal: &Principal, check: Check<'_>) -> Result<(), AccessError>;

    /// Makes a short-lived token for an actor.
    fn issue(&self, actor: &Actor) -> Result<IssuedToken, AccessError>;

    /// Writes relations, and returns a token of the permission system for later checks.
    async fn write(&self, relations: &[Relation]) -> Result<String, AccessError>;

    /// Deletes relations.
    async fn delete(&self, relations: &[Relation]) -> Result<(), AccessError>;

    /// Deletes all relations of an object.
    async fn delete_object(&self, object: &Object) -> Result<(), AccessError>;
}
