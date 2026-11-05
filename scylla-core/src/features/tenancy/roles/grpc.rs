//! The gRPC handler of `RoleService`: authenticate, parse, authorize on the owner of the role, then
//! call the service.

use std::sync::Arc;

use scylla_domain::authz::{Check, Object, Schema};
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::catalog::{self, Level};
use scylla_domain::tenancy::roles::{
    Holder, NewRole, RoleAdmin, RoleCaller, RoleHolder, RoleId, MAX_DESCRIPTION,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Gate};
use scylla_proto::access::v1 as pb;
use scylla_proto::access::v1::role_service_server::{RoleService, RoleServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::convert;
use super::service::{self, RoleFields};
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    schema: Schema,
) -> RoleServiceServer<RolesGrpc> {
    RoleServiceServer::new(RolesGrpc { pool, gate, schema })
}

/// The handler.
pub(super) struct RolesGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
    /// The permission model: what the roles of the schema give.
    schema: Schema,
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

    /// Handles `CreateRole`.
    async fn create(
        &self,
        request: Request<pb::CreateRoleRequest>,
    ) -> Result<pb::CreateRoleResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace = optional_workspace(request.workspace_id)?;
        let level = Level::parse(&request.level)
            .ok_or_else(|| Error::invalid("level", "is not a level"))?;
        let holder = Holder::parse(&request.holder)
            .ok_or_else(|| Error::invalid("holder", "is not a kind of holder"))?;
        if level == Level::Platform && (workspace.is_some() || holder == Holder::Apps) {
            let reason = "of a role of the platform level is the instance and people";
            return Err(Error::invalid("level", reason));
        }
        let fields = fields(request.name, request.description, request.permissions)?;

        // Check the route permission on the owner, then the permissions of the role.
        let check = match workspace {
            Some(workspace) => RoleHolder::CREATE_ROLE.on(workspace),
            None => RoleAdmin::CREATE_ROLE.on(Platform::ID),
        };
        self.gate.authorize(&caller, check).await?;
        let permissions = service::complete(level, holder, &fields.permissions)?;
        let object = owner_object(workspace);
        let gate = self.gate.as_ref();
        service::check_escalation(gate, &caller, &object, workspace, &permissions).await?;

        // Make the role.
        let new = NewRole {
            id: RoleId::generate(),
            workspace,
            name: fields.name,
            description: fields.description,
            level,
            holder,
            permissions,
            created_by: caller.actor.id,
        };
        let role = service::create(&self.pool, new).await?;
        Ok(pb::CreateRoleResponse {
            role: Some(role.to_proto()),
        })
    }

    /// Handles `UpdateRole`.
    async fn update(
        &self,
        request: Request<pb::UpdateRoleRequest>,
    ) -> Result<pb::UpdateRoleResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: RoleId = parse::required(request.role_id, "role_id")?;
        let fields = fields(request.name, request.description, request.permissions)?;

        // Check the route permission on the owner. An unknown role gets the answer of a refusal.
        let platform_check = RoleAdmin::UPDATE_ROLE.on(Platform::ID);
        let row = service::owner(&self.pool, id).await?;
        let check = match row.as_ref().and_then(|row| row.workspace) {
            Some(workspace) => RoleHolder::UPDATE_ROLE.on(workspace),
            None => platform_check.clone(),
        };
        let Some(row) = row else {
            return Err(AccessError::denied(&caller, &platform_check).into());
        };
        self.gate.authorize(&caller, check).await?;

        // Check the permissions of the role, then change it.
        let permissions = service::complete(row.level, row.holder, &fields.permissions)?;
        let object = owner_object(row.workspace);
        let gate = self.gate.as_ref();
        service::check_escalation(gate, &caller, &object, row.workspace, &permissions).await?;
        let role = service::update(&self.pool, id, &fields, &permissions).await?;
        Ok(pb::UpdateRoleResponse {
            role: Some(role.to_proto()),
        })
    }

    /// Handles `DeleteRole`.
    async fn delete(
        &self,
        request: Request<pb::DeleteRoleRequest>,
    ) -> Result<pb::DeleteRoleResponse, Error> {
        // Authenticate the caller, then read the role.
        let caller = self.gate.caller(&request).await?;
        let id: RoleId = parse::required(request.into_inner().role_id, "role_id")?;

        // Check the route permission on the owner. An unknown role gets the answer of a refusal.
        let platform_check: Check = RoleAdmin::DELETE_ROLE.on(Platform::ID);
        let row = service::owner(&self.pool, id).await?;
        let check = match row.as_ref().and_then(|row| row.workspace) {
            Some(workspace) => RoleHolder::DELETE_ROLE.on(workspace),
            None => platform_check.clone(),
        };
        if row.is_none() {
            return Err(AccessError::denied(&caller, &platform_check).into());
        }
        self.gate.authorize(&caller, check).await?;
        service::delete(&self.pool, id).await?;
        Ok(pb::DeleteRoleResponse {})
    }
}

/// Reads an optional workspace of a request.
fn optional_workspace(
    id: Option<scylla_proto::common::v1::Id>,
) -> Result<Option<WorkspaceId>, Error> {
    id.map(|id| parse::field(id, "workspace_id")).transpose()
}

/// Returns the object of the owner of a role.
fn owner_object(workspace: Option<WorkspaceId>) -> Object {
    match workspace {
        Some(workspace) => Object::of(workspace),
        None => Object::of(Platform::ID),
    }
}

/// Reads the fields of a custom role.
fn fields(
    name: String,
    description: String,
    permissions: Vec<String>,
) -> Result<RoleFields, Error> {
    let description = description.trim().to_owned();
    if description.chars().count() > MAX_DESCRIPTION {
        let reason = format!("must have at most {MAX_DESCRIPTION} characters");
        return Err(Error::invalid("description", reason));
    }
    Ok(RoleFields {
        name: parse::field(name, "name")?,
        description,
        permissions,
    })
}
