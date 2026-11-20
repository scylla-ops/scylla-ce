//! The gRPC handler of `RoleService`: authenticate, parse, authorize on the owner of the role, then
//! call the service.

use std::sync::Arc;

use scylla_domain::authz::Schema;
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::catalog;
use scylla_domain::tenancy::roles::{RoleAdmin, RoleCaller, RoleHolder};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Gate, Grants, Hooks};
use scylla_proto::access::v1 as pb;
use scylla_proto::access::v1::role_service_server::{RoleService, RoleServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::{convert, holders, service};
use crate::api::parse::{self, timestamp, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    schema: Schema,
    grants: Arc<Grants>,
    hooks: Arc<Hooks>,
) -> RoleServiceServer<RolesGrpc> {
    RoleServiceServer::new(RolesGrpc {
        pool,
        gate,
        schema,
        grants,
        hooks,
    })
}

/// The handler.
pub(super) struct RolesGrpc {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The access gate.
    pub(super) gate: Arc<dyn Gate>,
    /// The permission model: what the roles of the schema give.
    pub(super) schema: Schema,
    /// The kinds of objects with roles, for a move of the holders.
    pub(super) grants: Arc<Grants>,
    /// The hooks of the edition, for a membership.
    pub(super) hooks: Arc<Hooks>,
}

/// The answer of a route.
type Answer<T> = Result<Response<T>, Status>;

#[tonic::async_trait]
impl RoleService for RolesGrpc {
    async fn list_permission_catalog(
        &self,
        request: Request<pb::ListPermissionCatalogRequest>,
    ) -> Answer<pb::ListPermissionCatalogResponse> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        self.gate
            .authorize(
                &caller,
                RoleCaller::LIST_PERMISSION_CATALOG.on(caller.actor.id),
            )
            .await
            .map_err(Error::from)?;
        Ok(Response::new(pb::ListPermissionCatalogResponse {
            areas: catalog::areas().map(convert::area).collect(),
        }))
    }

    async fn list_roles(
        &self,
        request: Request<pb::ListRolesRequest>,
    ) -> Answer<pb::ListRolesResponse> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn get_role(&self, request: Request<pb::GetRoleRequest>) -> Answer<pb::GetRoleResponse> {
        Ok(Response::new(self.get(request).await?))
    }

    async fn create_role(
        &self,
        request: Request<pb::CreateRoleRequest>,
    ) -> Answer<pb::CreateRoleResponse> {
        Ok(Response::new(self.create(request).await?))
    }

    async fn update_role(
        &self,
        request: Request<pb::UpdateRoleRequest>,
    ) -> Answer<pb::UpdateRoleResponse> {
        Ok(Response::new(self.update(request).await?))
    }

    async fn list_role_holders(
        &self,
        request: Request<pb::ListRoleHoldersRequest>,
    ) -> Answer<pb::ListRoleHoldersResponse> {
        Ok(Response::new(self.holders(request).await?))
    }

    async fn delete_role(
        &self,
        request: Request<pb::DeleteRoleRequest>,
    ) -> Answer<pb::DeleteRoleResponse> {
        Ok(Response::new(self.delete(request).await?))
    }
}

impl RolesGrpc {
    /// Handles `ListRoles`.
    async fn list(
        &self,
        request: Request<pb::ListRolesRequest>,
    ) -> Result<pb::ListRolesResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let workspace = optional_workspace(request.into_inner().workspace_id)?;
        let check = match workspace {
            Some(workspace) => RoleHolder::LIST_ROLES.on(workspace),
            None => RoleAdmin::LIST_ROLES.on(Platform::ID),
        };
        self.gate.authorize(&caller, check).await?;
        let list = service::list(&self.pool, self.gate.as_ref(), &self.schema, workspace).await?;
        Ok(pb::ListRolesResponse {
            roles: list.into_iter().map(ToProto::to_proto).collect(),
        })
    }

    /// Handles `GetRole`. An unknown key gets the answer of a refusal.
    async fn get(
        &self,
        request: Request<pb::GetRoleRequest>,
    ) -> Result<pb::GetRoleResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace = optional_workspace(request.workspace_id)?;
        let check = match workspace {
            Some(workspace) => RoleHolder::GET_ROLE.on(workspace),
            None => RoleAdmin::GET_ROLE.on(Platform::ID),
        };
        self.gate.authorize(&caller, check.clone()).await?;
        let list = service::list(&self.pool, self.gate.as_ref(), &self.schema, workspace).await?;
        let role = list
            .into_iter()
            .find(|role| role.key == request.key)
            .ok_or_else(|| AccessError::denied(&caller, &check))?;
        Ok(pb::GetRoleResponse {
            role: Some(role.to_proto()),
        })
    }

    /// Handles `ListRoleHolders`. An unknown key gets the answer of a refusal.
    async fn holders(
        &self,
        request: Request<pb::ListRoleHoldersRequest>,
    ) -> Result<pb::ListRoleHoldersResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace = optional_workspace(request.workspace_id)?;
        let check = match workspace {
            Some(workspace) => RoleHolder::LIST_ROLE_HOLDERS.on(workspace),
            None => RoleAdmin::LIST_ROLE_HOLDERS.on(Platform::ID),
        };
        self.gate.authorize(&caller, check.clone()).await?;
        if !holders::known(&self.pool, &request.key, workspace).await? {
            return Err(AccessError::denied(&caller, &check).into());
        }
        let gate = self.gate.as_ref();
        let (entries, total) = holders::list(&self.pool, gate, &request.key, workspace).await?;
        Ok(pb::ListRoleHoldersResponse {
            holders: entries
                .into_iter()
                .map(|(profile, role)| pb::RoleHolderEntry {
                    actor_id: Some(profile.actor_id.to_proto()),
                    actor_kind: profile.actor_kind.as_str().to_owned(),
                    display_name: profile.display_name,
                    handle: profile.handle.unwrap_or_default(),
                    scope_kind: role.scope.level.as_str().to_owned(),
                    scope_id: Some(scylla_proto::common::v1::Id {
                        value: role.scope.id,
                    }),
                    scope_name: role.scope.name,
                    granted_at: role.granted_at.map(timestamp),
                    runner: profile.runner,
                })
                .collect(),
            total: u32::try_from(total).unwrap_or(u32::MAX),
        })
    }
}

/// Reads an optional workspace of a request.
pub(super) fn optional_workspace(
    id: Option<scylla_proto::common::v1::Id>,
) -> Result<Option<WorkspaceId>, Error> {
    id.map(|id| parse::field(id, "workspace_id")).transpose()
}
