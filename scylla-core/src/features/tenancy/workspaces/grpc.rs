//! The gRPC handler of `WorkspaceService`: authenticate, authorize, parse, call the service,
//! answer. The routes of the owners are in `owners.rs`.

use std::sync::Arc;

use scylla_domain::identity::users;
use scylla_domain::tenancy::catalog;
use scylla_domain::tenancy::slugs::Slug;
use scylla_domain::tenancy::workspaces::{
    ListWorkspaces, NewSlug, NewWorkspace, VisibleWorkspaces, Workspace, WorkspaceCaller,
    WorkspaceChange, WorkspaceId,
};
use scylla_domain::Name;
use scylla_extension::{AccessError, Gate, Grants, Hooks, Principal};
use scylla_proto::workspaces::v1 as pb;
use scylla_proto::workspaces::v1::get_workspace_request::Workspace as Named;
use scylla_proto::workspaces::v1::workspace_service_server::{
    WorkspaceService, WorkspaceServiceServer,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    hooks: Arc<Hooks>,
    grants: Arc<Grants>,
) -> WorkspaceServiceServer<WorkspacesGrpc> {
    WorkspaceServiceServer::new(WorkspacesGrpc {
        pool,
        gate,
        hooks,
        grants,
    })
}

/// The handler.
pub(super) struct WorkspacesGrpc {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The access gate.
    pub(super) gate: Arc<dyn Gate>,
    /// The hooks of the edition, for a new owner.
    pub(super) hooks: Arc<Hooks>,
    /// The kinds of objects with roles, for an owner that leaves.
    pub(super) grants: Arc<Grants>,
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

    async fn update_workspace(
        &self,
        request: Request<pb::UpdateWorkspaceRequest>,
    ) -> Result<Response<pb::UpdateWorkspaceResponse>, Status> {
        Ok(Response::new(self.update(request).await?))
    }

    async fn delete_workspace(
        &self,
        request: Request<pb::DeleteWorkspaceRequest>,
    ) -> Result<Response<pb::DeleteWorkspaceResponse>, Status> {
        Ok(Response::new(self.delete(request).await?))
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
    /// Handles `CreateWorkspace`. The caller becomes the first owner, so it must be a user. With
    /// no slug, the slug comes from the name.
    async fn create(
        &self,
        request: Request<pb::CreateWorkspaceRequest>,
    ) -> Result<pb::CreateWorkspaceResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let name: Name = parse::field(request.name, "name")?;
        let slug = match request.slug {
            Some(slug) => NewSlug::Chosen(parse::field(slug, "slug")?),
            None => NewSlug::Derived(Slug::derive(name.as_str())),
        };
        let new = NewWorkspace {
            creator: caller.actor.id,
            name,
            slug,
        };

        // Check the route permission on the caller. Only a user can own a workspace.
        let check = WorkspaceCaller::CREATE_WORKSPACE.on(caller.actor.id);
        self.gate.authorize(&caller, check.clone()).await?;
        if caller.actor.kind != users::KIND {
            let reason = "only a user can own a workspace".to_owned();
            let object = check.object();
            return Err(AccessError::refused(&caller, check.permission(), object, reason).into());
        }

        // Create the workspace.
        let workspace = service::create(&self.pool, new).await?;
        Ok(pb::CreateWorkspaceResponse {
            workspace: Some(workspace.to_proto()),
        })
    }

    /// Handles `GetWorkspace`, by id or by slug. An unknown workspace gives
    /// `PERMISSION_DENIED`, as a refusal does. A slug is public: it is in the URLs.
    async fn get(
        &self,
        request: Request<pb::GetWorkspaceRequest>,
    ) -> Result<pb::GetWorkspaceResponse, Error> {
        // Authenticate the caller, then read the id or the slug.
        let caller = self.gate.caller(&request).await?;
        let named = request
            .into_inner()
            .workspace
            .ok_or_else(|| Error::invalid("workspace", "is required"))?;

        // Find the workspace. An unknown one gets the answer of a refusal.
        let workspace = match named {
            Named::WorkspaceId(id) => {
                let id: WorkspaceId = parse::field(id, "workspace_id")?;
                let check = Workspace::GET_WORKSPACE.on(id);
                let Some(workspace) = service::get(&self.pool, id).await? else {
                    return Err(AccessError::denied(&caller, &check).into());
                };
                self.gate.authorize(&caller, check).await?;
                workspace
            }
            Named::Slug(slug) => self.get_by_slug(&caller, slug).await?,
        };
        Ok(pb::GetWorkspaceResponse {
            workspace: Some(workspace.to_proto()),
        })
    }

    /// Finds a workspace by its slug and checks the route permission on it. A refusal names the
    /// slug, not the id of the workspace.
    async fn get_by_slug(&self, caller: &Principal, slug: String) -> Result<Workspace, Error> {
        // Read the slug, then find its workspace.
        let slug: Slug = parse::field(slug, "slug")?;
        let permission = Workspace::GET_WORKSPACE.name();
        let reason = format!("{permission} on the workspace {} is denied", slug.as_str());
        let Some(workspace) = service::get_by_slug(&self.pool, &slug).await? else {
            let object = format!("workspace slug {}", slug.as_str());
            return Err(AccessError::refused(caller, permission, &object, reason).into());
        };

        // Check the route permission on it, and hide its id in a refusal.
        let check = Workspace::GET_WORKSPACE.on(workspace.id);
        match self.gate.authorize(caller, check).await {
            Ok(()) => Ok(workspace),
            Err(AccessError::Denied(_)) => Err(AccessError::Denied(reason).into()),
            Err(error) => Err(error.into()),
        }
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
        let all = self
            .gate
            .holds_on_instance(&caller, catalog::VIEW_ALL_WORKSPACES)
            .await?;
        let visible = VisibleWorkspaces {
            caller: caller.actor.id,
            granted,
            all,
        };
        let page = service::list(&self.pool, query, visible).await?;
        Ok(pb::ListWorkspacesResponse {
            workspaces: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `UpdateWorkspace`: a new name or a new slug. An unknown workspace gets the answer
    /// of a refusal.
    async fn update(
        &self,
        request: Request<pb::UpdateWorkspaceRequest>,
    ) -> Result<pb::UpdateWorkspaceResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let change = WorkspaceChange {
            name: request
                .name
                .map(|name| parse::field(name, "name"))
                .transpose()?,
            slug: request
                .slug
                .map(|slug| parse::field(slug, "slug"))
                .transpose()?,
            personal_tokens_enabled: request.personal_tokens_enabled,
        };

        // Check the route permission on a workspace that exists, then change it.
        let check = Workspace::UPDATE_WORKSPACE.on(id);
        if service::get(&self.pool, id).await?.is_none() {
            return Err(AccessError::denied(&caller, &check).into());
        }
        self.gate.authorize(&caller, check).await?;
        let workspace = service::update(&self.pool, id, change).await?;
        Ok(pb::UpdateWorkspaceResponse {
            workspace: Some(workspace.to_proto()),
        })
    }

    /// Handles `DeleteWorkspace`. Only an owner passes: no role gives this permission. An
    /// unknown workspace has no owner, so it gets the answer of a refusal.
    async fn delete(
        &self,
        request: Request<pb::DeleteWorkspaceRequest>,
    ) -> Result<pb::DeleteWorkspaceResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let id: WorkspaceId = parse::required(request.into_inner().workspace_id, "workspace_id")?;

        // Check the route permission, then delete the workspace.
        self.gate
            .authorize(&caller, Workspace::DELETE_WORKSPACE.on(id))
            .await?;
        service::delete(&self.pool, self.gate.as_ref(), id).await?;
        Ok(pb::DeleteWorkspaceResponse {})
    }
}
