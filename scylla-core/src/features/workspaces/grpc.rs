//! The gRPC handler of `WorkspaceService`: authenticate, authorize, parse, call the service,
//! answer.

use std::sync::Arc;

use scylla_domain::actors::ActorId;
use scylla_domain::users;
use scylla_domain::workspaces::{
    ListOwners, ListWorkspaces, NewWorkspace, VisibleWorkspaces, Workspace, WorkspaceCaller,
    WorkspaceId, WorkspaceOwner,
};
use scylla_extension::{AccessError, Gate};
use scylla_proto::workspaces::v1 as pb;
use scylla_proto::workspaces::v1::workspace_service_server::{
    WorkspaceService, WorkspaceServiceServer,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(pool: PgPool, gate: Arc<dyn Gate>) -> WorkspaceServiceServer<WorkspacesGrpc> {
    WorkspaceServiceServer::new(WorkspacesGrpc { pool, gate })
}

/// The handler.
pub(super) struct WorkspacesGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl WorkspaceService for WorkspacesGrpc {
    async fn create_workspace(
        &self,
        request: Request<pb::CreateWorkspaceRequest>,
    ) -> Result<Response<pb::CreateWorkspaceResponse>, Status> {
        Ok(Response::new(self.create(request).await?))
    }

    async fn get_workspace(
        &self,
        request: Request<pb::GetWorkspaceRequest>,
    ) -> Result<Response<pb::GetWorkspaceResponse>, Status> {
        Ok(Response::new(self.get(request).await?))
    }

    async fn list_workspaces(
        &self,
        request: Request<pb::ListWorkspacesRequest>,
    ) -> Result<Response<pb::ListWorkspacesResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn add_owner(
        &self,
        request: Request<pb::AddOwnerRequest>,
    ) -> Result<Response<pb::AddOwnerResponse>, Status> {
        Ok(Response::new(self.add(request).await?))
    }

    async fn remove_owner(
        &self,
        request: Request<pb::RemoveOwnerRequest>,
    ) -> Result<Response<pb::RemoveOwnerResponse>, Status> {
        Ok(Response::new(self.remove(request).await?))
    }

    async fn list_owners(
        &self,
        request: Request<pb::ListOwnersRequest>,
    ) -> Result<Response<pb::ListOwnersResponse>, Status> {
        Ok(Response::new(self.owners(request).await?))
    }
}

impl WorkspacesGrpc {
    /// Handles `CreateWorkspace`. The caller becomes the first owner, so it must be a user.
    async fn create(
        &self,
        request: Request<pb::CreateWorkspaceRequest>,
    ) -> Result<pb::CreateWorkspaceResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let new = NewWorkspace {
            creator: caller.actor.id,
            name: parse::field(request.into_inner().name, "name")?,
        };

        // Check the route permission on the caller. Only a user can own a workspace.
        self.gate
            .authorize(
                &caller,
                WorkspaceCaller::CREATE_WORKSPACE.on(caller.actor.id),
            )
            .await?;
        if caller.actor.kind != users::KIND {
            let reason = "only a user can own a workspace".to_owned();
            return Err(AccessError::Denied(reason).into());
        }

        // Create the workspace.
        let workspace = service::create(&self.pool, new).await?;
        Ok(pb::CreateWorkspaceResponse {
            workspace: Some(workspace.to_proto()),
        })
    }

    /// Handles `GetWorkspace`. An unknown workspace gives `PERMISSION_DENIED`, so no caller
    /// learns which ids exist.
    async fn get(
        &self,
        request: Request<pb::GetWorkspaceRequest>,
    ) -> Result<pb::GetWorkspaceResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let id: WorkspaceId = parse::required(request.into_inner().workspace_id, "workspace_id")?;

        // Check the route permission. An unknown workspace gets the answer of a refusal.
        let check = Workspace::GET_WORKSPACE.on(id);
        let Some(workspace) = service::get(&self.pool, id).await? else {
            return Err(AccessError::denied(&check).into());
        };
        self.gate.authorize(&caller, check).await?;
        Ok(pb::GetWorkspaceResponse {
            workspace: Some(workspace.to_proto()),
        })
    }

    /// Handles `ListWorkspaces`: the workspaces that the caller owns or where it has a role.
    async fn list(
        &self,
        request: Request<pb::ListWorkspacesRequest>,
    ) -> Result<pb::ListWorkspacesResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let query = ListWorkspaces {
            page: parse::optional(request.into_inner().page, "page")?,
        };

        // Check the route permission on the caller: the list shows only what it can read.
        self.gate
            .authorize(
                &caller,
                WorkspaceCaller::LIST_WORKSPACES.on(caller.actor.id),
            )
            .await?;

        // Find the workspaces where the caller has a role. A failed lookup gives none.
        let granted = self
            .gate
            .granted(&caller, Workspace::LIST_WORKSPACES)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "a lookup failed: the list shows only the owned workspaces");
                Vec::new()
            });

        // Read one page of these workspaces and of the workspaces that the caller owns.
        let visible = VisibleWorkspaces {
            caller: caller.actor.id,
            granted,
        };
        let page = service::list(&self.pool, query, visible).await?;
        Ok(pb::ListWorkspacesResponse {
            workspaces: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `AddOwner`. Only an owner passes: no role gives this permission. An unknown
    /// workspace gets the answer of a refusal.
    async fn add(
        &self,
        request: Request<pb::AddOwnerRequest>,
    ) -> Result<pb::AddOwnerResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let user: ActorId = parse::required(request.user_id, "user_id")?;

        // Check the route permission, then add the owner.
        self.gate
            .authorize(&caller, Workspace::ADD_OWNER.on(workspace))
            .await?;
        service::add_owner(&self.pool, workspace, user).await?;
        Ok(pb::AddOwnerResponse {})
    }

    /// Handles `RemoveOwner`. Only an owner passes, and the last owner stays.
    async fn remove(
        &self,
        request: Request<pb::RemoveOwnerRequest>,
    ) -> Result<pb::RemoveOwnerResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let user: ActorId = parse::required(request.user_id, "user_id")?;

        // Check the route permission, then remove the owner.
        self.gate
            .authorize(&caller, Workspace::REMOVE_OWNER.on(workspace))
            .await?;
        service::remove_owner(&self.pool, workspace, user).await?;
        Ok(pb::RemoveOwnerResponse {})
    }

    /// Handles `ListOwners`. A reader of the workspace passes.
    async fn owners(
        &self,
        request: Request<pb::ListOwnersRequest>,
    ) -> Result<pb::ListOwnersResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let query = ListOwners {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission, then read one page of owners.
        self.gate
            .authorize(&caller, Workspace::LIST_OWNERS.on(query.workspace_id))
            .await?;
        let page = service::owners(&self.pool, query).await?;
        Ok(pb::ListOwnersResponse {
            owners: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }
}

impl ToProto<pb::Workspace> for Workspace {
    fn to_proto(self) -> pb::Workspace {
        pb::Workspace {
            id: Some(self.id.to_proto()),
            name: self.name,
            created_at: Some(parse::timestamp(self.created_at)),
        }
    }
}

impl ToProto<pb::WorkspaceOwner> for WorkspaceOwner {
    fn to_proto(self) -> pb::WorkspaceOwner {
        pb::WorkspaceOwner {
            user_id: Some(self.user_id.to_proto()),
            display_name: self.display_name,
            since: Some(parse::timestamp(self.since)),
        }
    }
}
