//! The gRPC routes that change a custom role: `CreateRole`, `UpdateRole` and `DeleteRole`.

use scylla_domain::authz::{Check, Object};
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::catalog::Level;
use scylla_domain::tenancy::roles::{
    Holder, NewRole, RoleAdmin, RoleHolder, RoleId, MAX_DESCRIPTION,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::AccessError;
use scylla_proto::access::v1 as pb;
use tonic::Request;

use super::grpc::{optional_workspace, RolesGrpc};
use super::moving::Mover;
use super::service::{self, RoleFields};
use crate::api::parse::{self, ToProto};
use crate::Error;

impl RolesGrpc {
    /// Handles `CreateRole`.
    pub(super) async fn create(
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
    pub(super) async fn update(
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
    pub(super) async fn delete(
        &self,
        request: Request<pb::DeleteRoleRequest>,
    ) -> Result<pb::DeleteRoleResponse, Error> {
        // Authenticate the caller, then read the role.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: RoleId = parse::required(request.role_id, "role_id")?;

        // Check the route permission on the owner. An unknown role gets the answer of a refusal.
        let platform_check: Check = RoleAdmin::DELETE_ROLE.on(Platform::ID);
        let row = service::owner(&self.pool, id).await?;
        let check = match row.as_ref().and_then(|row| row.workspace) {
            Some(workspace) => RoleHolder::DELETE_ROLE.on(workspace),
            None => platform_check.clone(),
        };
        let Some(row) = row else {
            return Err(AccessError::denied(&caller, &platform_check).into());
        };
        self.gate.authorize(&caller, check).await?;
        if request.move_to.is_empty() {
            service::delete(&self.pool, id).await?;
        } else {
            let mover = Mover {
                pool: &self.pool,
                gate: self.gate.as_ref(),
                schema: &self.schema,
                grants: &self.grants,
                hooks: &self.hooks,
            };
            mover
                .delete_with_move(&caller, &row, &request.move_to)
                .await?;
        }
        Ok(pb::DeleteRoleResponse {})
    }
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
