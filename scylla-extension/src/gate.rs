//! Access control: each request proves its actor with a token, then passes permission checks.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use scylla_domain::authz::{Check, Object, Permission, Relationship, Resource};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::apps::AppKeyId;
use scylla_domain::identity::users::SessionId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
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
    /// What the token is bound to.
    binding: Binding,
    /// The end of the validity of the token.
    expires_at: OffsetDateTime,
}

impl Principal {
    /// Makes a principal. Only a gate makes one.
    pub fn new(
        actor: Actor,
        operation: Operation,
        token: Arc<dyn Any + Send + Sync>,
        binding: Binding,
        expires_at: OffsetDateTime,
    ) -> Self {
        Self {
            actor,
            operation,
            token,
            binding,
            expires_at,
        }
    }

    /// Returns the session of the token, if the actor signed in as a user.
    pub fn session(&self) -> Option<SessionId> {
        match self.binding {
            Binding::Session(session) => Some(session),
            _ => None,
        }
    }

    /// Returns the end of the validity of the token.
    pub fn expires_at(&self) -> OffsetDateTime {
        self.expires_at
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

/// What a token is bound to. The token stops when its session closes or its key is revoked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// Nothing: a token of the command line.
    None,
    /// The session of a user that signed in.
    Session(SessionId),
    /// The key of an App that signed an assertion.
    Key(AppKeyId),
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
    /// Logs the refusal of a check at WARN and makes its answer. A route gives the same refusal
    /// for an object that does not exist, so no caller learns which ids exist.
    pub fn denied(principal: &Principal, check: &Check) -> Self {
        log_refusal(principal, check.permission(), check.object(), None);
        Self::Denied(format!("{check} is denied"))
    }

    /// Logs a refusal at WARN, with the actor, the route, the permission, the object and the
    /// reason, and makes its answer with the reason.
    pub fn refused(
        principal: &Principal,
        permission: &str,
        object: &dyn fmt::Display,
        reason: String,
    ) -> Self {
        log_refusal(principal, permission, object, Some(&reason));
        Self::Denied(reason)
    }
}

/// Logs the event `access.refused` at WARN. The log of the server puts it on the line of its
/// call.
fn log_refusal(
    principal: &Principal,
    permission: &str,
    object: &dyn fmt::Display,
    reason: Option<&str>,
) {
    tracing::warn!(
        event = "access.refused",
        actor = %principal.actor.id,
        actor_kind = principal.actor.kind.as_str(),
        operation = principal.operation().as_str(),
        permission,
        %object,
        reason,
        "access refused"
    );
}

/// A role of an actor on an object, as the permission system holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoleHolder {
    /// The name of the role.
    pub role: String,
    /// The actor.
    pub actor: ActorId,
}

/// One page of the roles on an object, in the order of the permission system.
#[derive(Clone, Debug, Default)]
pub struct RolePage {
    /// The roles of the page.
    pub holders: Vec<RoleHolder>,
    /// `true` if more roles come after the page.
    pub more: bool,
    /// The cursor after the last role of the page, if the page has one.
    pub cursor: Option<String>,
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

    /// Makes a short-lived token for an actor, bound to the session of a user that signed in or
    /// to the key of an App. A token of a revoked session or key stops at once.
    fn issue(&self, actor: &Actor, binding: Binding) -> Result<IssuedToken, AccessError>;

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

    /// Returns up to `count` roles on an object, after the cursor of a previous page or from the
    /// first role. `None` means that the permission system refuses the cursor.
    async fn roles(
        &self,
        object: &Object,
        cursor: Option<&str>,
        count: usize,
    ) -> Result<Option<RolePage>, AccessError>;

    /// Returns, for each permission, `true` if the actor of the principal has it on an object,
    /// with its workspace if it is in one. The rights are those of a route check: the actor
    /// itself, an instance admin on the platform, an owner of the workspace, else the roles on
    /// the object and on its workspace, in one call. It is not a route check: it records nothing
    /// on the operation, and the token does not take part.
    async fn permissions(
        &self,
        principal: &Principal,
        object: &Object,
        workspace: Option<WorkspaceId>,
        permissions: &[&'static str],
    ) -> Result<Vec<bool>, AccessError>;

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
