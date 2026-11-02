//! The gRPC handler of `AccessService`: authenticate, read the role, authorize on the object,
//! then write or delete the relationship in SpiceDB.

use std::sync::Arc;

use scylla_domain::actors::ActorId;
use scylla_domain::authz::{Check, Relationship};
use scylla_extension::{AccessError, Gate, Grantable, Grants, Principal};
use scylla_proto::access::v1 as pb;
use scylla_proto::access::v1::access_service_server::{AccessService, AccessServiceServer};
use scylla_proto::common::v1::Id;
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use crate::features::actors;
use crate::parse;
use crate::Error;

/// Makes the check of a route on an object: `Grantable::grant` or `Grantable::revoke`.
type RouteCheck = fn(&Grantable, &str) -> Option<Check>;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    grants: Arc<Grants>,
) -> AccessServiceServer<GrantsGrpc> {
    AccessServiceServer::new(GrantsGrpc { pool, gate, grants })
}

/// The handler.
pub(super) struct GrantsGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
    /// The roles that the service can give.
    grants: Arc<Grants>,
}

#[tonic::async_trait]
impl AccessService for GrantsGrpc {
    async fn grant(
        &self,
        request: Request<pb::GrantRequest>,
    ) -> Result<Response<pb::GrantResponse>, Status> {
        Ok(Response::new(self.give(request).await?))
    }

    async fn revoke(
        &self,
        request: Request<pb::RevokeRequest>,
    ) -> Result<Response<pb::RevokeResponse>, Status> {
        Ok(Response::new(self.take_back(request).await?))
    }
}

impl GrantsGrpc {
    /// Handles `Grant`. The actor that gets the role must exist.
    async fn give(&self, request: Request<pb::GrantRequest>) -> Result<pb::GrantResponse, Error> {
        // Authenticate the caller, then pass the route check on the object of the role.
        let caller = self.gate.caller(&request).await?;
        let pb::GrantRequest {
            object,
            role,
            actor_id,
        } = request.into_inner();
        let (actor, relationship) = self
            .authorize(&caller, Grantable::grant, object, &role, actor_id)
            .await?;

        // Give the role to an actor that exists.
        actors::find(&self.pool, actor).await?;
        self.gate.write(&[relationship]).await?;
        Ok(pb::GrantResponse {})
    }

    /// Handles `Revoke`. A role that the actor does not have is not an error.
    async fn take_back(
        &self,
        request: Request<pb::RevokeRequest>,
    ) -> Result<pb::RevokeResponse, Error> {
        // Authenticate the caller, then pass the route check on the object of the role.
        let caller = self.gate.caller(&request).await?;
        let pb::RevokeRequest {
            object,
            role,
            actor_id,
        } = request.into_inner();
        let (_, relationship) = self
            .authorize(&caller, Grantable::revoke, object, &role, actor_id)
            .await?;

        // Take back the role.
        self.gate.delete(&[relationship]).await?;
        Ok(pb::RevokeResponse {})
    }

    /// Reads the role of a request and passes the route check on its object, with the workspace
    /// of the object. Returns the actor and the relationship of the role.
    async fn authorize(
        &self,
        caller: &Principal,
        route: RouteCheck,
        object: Option<pb::ObjectRef>,
        role: &str,
        actor_id: Option<Id>,
    ) -> Result<(ActorId, Relationship), Error> {
        // Read the kind, the object, the actor and the role. Each bad field is a request error.
        let object = object.ok_or_else(|| Error::invalid("object", "is required"))?;
        let kind = self
            .grants
            .get(&object.kind)
            .ok_or_else(|| Error::invalid("object.kind", "has no roles"))?;
        let id = object
            .id
            .map(|id| id.value)
            .ok_or_else(|| Error::invalid("object.id", "is required"))?;
        let check = route(kind, &id).ok_or_else(|| Error::invalid("object.id", "is not an id"))?;
        let actor: ActorId = parse::required(actor_id, "actor_id")?;
        let relationship = kind
            .relationship(role, &id, actor)
            .ok_or_else(|| Error::invalid("role", format!("is not a role of {}", object.kind)))?;

        // Name the workspace of the object. An unknown object gets the answer of a refusal.
        let check = match kind.workspace(self.pool.clone(), &id) {
            None => check,
            Some(search) => match search.await? {
                Some(workspace) => check.within(workspace),
                None => return Err(AccessError::denied(&check).into()),
            },
        };

        // Check the route permission on the object.
        self.gate.authorize(caller, check).await?;
        Ok((actor, relationship))
    }
}
