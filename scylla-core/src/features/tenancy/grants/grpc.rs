//! The gRPC handler of `AccessService`: authenticate, read the object, authorize on it with its
//! workspace, then act. `Grant` and `Revoke` are here, `ListAccess` is in `list.rs`, and
//! `CheckPermissions` in `check.rs`.

use std::sync::Arc;

use scylla_domain::authz::{Check, Resource, Schema};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users;
use scylla_domain::platform::Platform;
use scylla_domain::projects::Project;
use scylla_domain::tenancy::grants::OWNER;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{AccessError, Gate, Grantable, Grants, Hooks, Principal};
use scylla_proto::access::v1 as pb;
use scylla_proto::access::v1::access_service_server::{AccessService, AccessServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::{give, membership, service};
use crate::api::parse;
use crate::Error;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    grants: Arc<Grants>,
    schema: Schema,
    hooks: Arc<Hooks>,
) -> AccessServiceServer<GrantsGrpc> {
    AccessServiceServer::new(GrantsGrpc {
        pool,
        gate,
        grants,
        schema,
        hooks,
    })
}

/// The handler.
pub(super) struct GrantsGrpc {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The access gate.
    pub(super) gate: Arc<dyn Gate>,
    /// The kinds of objects that the service serves, with their roles.
    pub(super) grants: Arc<Grants>,
    /// The permission model: the permissions of each definition.
    pub(super) schema: Schema,
    /// The hooks of the edition, for a membership.
    pub(super) hooks: Arc<Hooks>,
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

    async fn list_access(
        &self,
        request: Request<pb::ListAccessRequest>,
    ) -> Result<Response<pb::ListAccessResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn list_members(
        &self,
        request: Request<pb::ListMembersRequest>,
    ) -> Result<Response<pb::ListMembersResponse>, Status> {
        Ok(Response::new(self.members(request).await?))
    }

    async fn get_member_access(
        &self,
        request: Request<pb::GetMemberAccessRequest>,
    ) -> Result<Response<pb::GetMemberAccessResponse>, Status> {
        Ok(Response::new(self.member_access(request).await?))
    }

    async fn list_held_permissions(
        &self,
        request: Request<pb::ListHeldPermissionsRequest>,
    ) -> Result<Response<pb::ListHeldPermissionsResponse>, Status> {
        Ok(Response::new(self.held_permissions(request).await?))
    }

    async fn check_permissions(
        &self,
        request: Request<pb::CheckPermissionsRequest>,
    ) -> Result<Response<pb::CheckPermissionsResponse>, Status> {
        Ok(Response::new(self.check(request).await?))
    }
}

impl GrantsGrpc {
    /// Handles `Grant`. The actor that gets the role must exist, and an agent gets a role only
    /// in its own workspace.
    async fn give(&self, request: Request<pb::GrantRequest>) -> Result<pb::GrantResponse, Error> {
        // Authenticate the caller, then read the object, the actor and the role.
        let caller = self.gate.caller(&request).await?;
        let pb::GrantRequest {
            object,
            role,
            actor_id,
        } = request.into_inner();
        if let Some(object) = on_platform(&object) {
            return self.give_on_platform(&caller, object, role, actor_id).await;
        }
        let (kind, name, id) = self.target(object)?;
        if name == Workspace::NAME && role == OWNER {
            return self.give_owner(caller, kind, &id, actor_id).await;
        }
        let check = kind.grant(&id).ok_or_else(not_an_id)?;
        let actor: ActorId = parse::required(actor_id, "actor_id")?;
        let given = give::read_name(kind, &name, &id, &role, actor)?;

        // Check the route permission on the object, with its workspace.
        let (check, workspace) = self.within(&caller, kind, &id, check).await?;
        self.gate.authorize(&caller, check).await?;

        // Check who gets the role and who gives it.
        let given = self.resolve(given, &name, workspace).await?;
        let grantee = service::check_actor(&self.pool, actor, workspace).await?;
        give::check_holder(&given, &name, &role, &grantee.kind)?;
        if name == Project::NAME && grantee.kind == users::KIND {
            membership::check_in_workspace(&self.pool, self.gate.as_ref(), actor, workspace)
                .await?;
        }
        self.check_can_give(&caller, &given, (&name, &id), workspace, &role)
            .await?;

        // Give the role, after the hooks of a user that joins.
        let by = caller.actor.id;
        let object = format!("{name}:{id}");
        if grantee.kind == users::KIND {
            let member = (actor, object.clone(), role.as_str());
            service::join(&self.pool, &self.hooks, caller.actor, workspace, member).await?;
        }
        give::write(
            &self.pool,
            self.gate.as_ref(),
            given,
            (&name, &id),
            actor,
            by,
        )
        .await?;
        tracing::info!(event = "role.granted", role, object, grantee = %actor);
        Ok(pb::GrantResponse {})
    }

    /// Handles `Revoke`. A role that the actor does not have is not an error.
    async fn take_back(
        &self,
        request: Request<pb::RevokeRequest>,
    ) -> Result<pb::RevokeResponse, Error> {
        // Authenticate the caller, then read the object, the actor and the role.
        let caller = self.gate.caller(&request).await?;
        let pb::RevokeRequest {
            object,
            role,
            actor_id,
        } = request.into_inner();
        if let Some(object) = on_platform(&object) {
            return self
                .take_back_on_platform(&caller, object, role, actor_id)
                .await;
        }
        let (kind, name, id) = self.target(object)?;
        if name == Workspace::NAME && role == OWNER {
            return self.take_back_owner(caller, kind, &id, actor_id).await;
        }
        let check = kind.revoke(&id).ok_or_else(not_an_id)?;
        let actor: ActorId = parse::required(actor_id, "actor_id")?;
        let given = give::read_name(kind, &name, &id, &role, actor)?;

        // Check the route permission on the object with its workspace, then take back the role.
        let (check, workspace) = self.within(&caller, kind, &id, check).await?;
        self.gate.authorize(&caller, check).await?;
        let given = self.resolve(given, &name, workspace).await?;
        give::erase(&self.pool, self.gate.as_ref(), given, (&name, &id), actor).await?;
        let object = format!("{name}:{id}");
        let member = (actor, object.clone(), role.as_str());
        let by = caller.actor;
        service::leave(&self.pool, &self.hooks, by.clone(), workspace, member).await?;
        tracing::info!(event = "role.revoked", role, object, grantee = %actor);

        // A user with no role left in the workspace loses the roles on its projects.
        if name == Workspace::NAME {
            let leaving = membership::Leaving {
                pool: &self.pool,
                gate: self.gate.as_ref(),
                grants: &self.grants,
                hooks: &self.hooks,
            };
            membership::end_membership(&leaving, by, actor, workspace).await?;
        }
        Ok(pb::RevokeResponse {})
    }

    /// Reads the object of a request: a kind with roles, its name, and the text of its id.
    pub(super) fn target(
        &self,
        object: Option<pb::ObjectRef>,
    ) -> Result<(&Grantable, String, String), Error> {
        let object = object.ok_or_else(|| Error::invalid("object", "is required"))?;
        let kind = self
            .grants
            .get(&object.kind)
            .ok_or_else(|| Error::invalid("object.kind", "has no roles"))?;
        let id = object
            .id
            .map(|id| id.value)
            .ok_or_else(|| Error::invalid("object.id", "is required"))?;
        Ok((kind, object.kind, id))
    }

    /// Finds the workspace of an object and names it in the check. An unknown object gets the
    /// answer of a refusal.
    pub(super) async fn within(
        &self,
        caller: &Principal,
        kind: &Grantable,
        id: &str,
        check: Check,
    ) -> Result<(Check, WorkspaceId), Error> {
        let search = kind
            .workspace(self.pool.clone(), id)
            .ok_or_else(not_an_id)?;
        match search.await? {
            Some(workspace) => Ok((check.within(workspace), workspace)),
            None => Err(AccessError::denied(caller, &check).into()),
        }
    }
}

/// Returns the object of a request if it is the platform.
fn on_platform(object: &Option<pb::ObjectRef>) -> Option<pb::ObjectRef> {
    object
        .as_ref()
        .filter(|object| object.kind == Platform::NAME)
        .cloned()
}

/// The error of an object id that is not an id.
pub(super) fn not_an_id() -> Error {
    Error::invalid("object.id", "is not an id")
}
