//! The gRPC handler of `ProjectGroupService`: authenticate, parse, authorize, call the service,
//! answer.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::projects::groups::{
    ListProjectGroups, NewProjectGroup, ProjectGroup, ProjectGroupAccess, ProjectGroupChange,
    ProjectGroupHolder, ProjectGroupId, ProjectMove,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_extension::{AccessError, Gate, Hooks, Principal};
use scylla_proto::projects::v1 as pb;
use scylla_proto::projects::v1::project_group_service_server::{
    ProjectGroupService, ProjectGroupServiceServer,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    hooks: Arc<Hooks>,
    gate: Arc<dyn Gate>,
) -> ProjectGroupServiceServer<ProjectGroupsGrpc> {
    ProjectGroupServiceServer::new(ProjectGroupsGrpc { pool, hooks, gate })
}

/// The handler.
pub(super) struct ProjectGroupsGrpc {
    /// The database pool.
    pool: PgPool,
    /// The hooks of the edition.
    hooks: Arc<Hooks>,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl ProjectGroupService for ProjectGroupsGrpc {
    async fn list_project_groups(
        &self,
        request: Request<pb::ListProjectGroupsRequest>,
    ) -> Result<Response<pb::ListProjectGroupsResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn create_project_group(
        &self,
        request: Request<pb::CreateProjectGroupRequest>,
    ) -> Result<Response<pb::CreateProjectGroupResponse>, Status> {
        Ok(Response::new(self.create(request).await?))
    }

    async fn update_project_group(
        &self,
        request: Request<pb::UpdateProjectGroupRequest>,
    ) -> Result<Response<pb::UpdateProjectGroupResponse>, Status> {
        Ok(Response::new(self.update(request).await?))
    }

    async fn delete_project_group(
        &self,
        request: Request<pb::DeleteProjectGroupRequest>,
    ) -> Result<Response<pb::DeleteProjectGroupResponse>, Status> {
        Ok(Response::new(self.delete(request).await?))
    }

    async fn move_project(
        &self,
        request: Request<pb::MoveProjectRequest>,
    ) -> Result<Response<pb::MoveProjectResponse>, Status> {
        Ok(Response::new(self.move_to(request).await?))
    }
}

impl ProjectGroupsGrpc {
    /// Handles `ListProjectGroups`: the groups of a workspace, in their order.
    async fn list(
        &self,
        request: Request<pb::ListProjectGroupsRequest>,
    ) -> Result<pb::ListProjectGroupsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let query = ListProjectGroups {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission on the workspace, then read one page.
        let check = ProjectGroupHolder::LIST_PROJECT_GROUPS.on(query.workspace_id);
        self.gate.authorize(&caller, check).await?;
        let page = service::list(&self.pool, query).await?;
        Ok(pb::ListProjectGroupsResponse {
            groups: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `CreateProjectGroup`, in a workspace.
    async fn create(
        &self,
        request: Request<pb::CreateProjectGroupRequest>,
    ) -> Result<pb::CreateProjectGroupResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let new = NewProjectGroup {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            created_by: caller.actor.id,
            name: parse::field(request.name, "name")?,
        };

        // Check the route permission on the workspace, then create the group.
        let check = ProjectGroupHolder::CREATE_PROJECT_GROUP.on(new.workspace_id);
        self.gate.authorize(&caller, check).await?;
        let group = service::create(&self.pool, &self.hooks, caller.actor, new).await?;
        Ok(pb::CreateProjectGroupResponse {
            group: Some(group.to_proto()),
        })
    }

    /// Handles `UpdateProjectGroup`: a new name or a new place. An unknown group gives
    /// `PERMISSION_DENIED`.
    async fn update(
        &self,
        request: Request<pb::UpdateProjectGroupRequest>,
    ) -> Result<pb::UpdateProjectGroupResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: ProjectGroupId = parse::required(request.group_id, "group_id")?;
        let change = ProjectGroupChange {
            name: request
                .name
                .map(|name| parse::field(name, "name"))
                .transpose()?,
            position: request
                .position
                .map(|position| usize::try_from(position).unwrap_or(usize::MAX)),
        };

        // Check the route permission, then change the group.
        let route = ProjectGroupHolder::UPDATE_PROJECT_GROUP;
        self.authorize_group(&caller, route, id).await?;
        let group = service::update(&self.pool, id, change).await?;
        Ok(pb::UpdateProjectGroupResponse {
            group: Some(group.to_proto()),
        })
    }

    /// Handles `DeleteProjectGroup`. An unknown group gives `PERMISSION_DENIED`.
    async fn delete(
        &self,
        request: Request<pb::DeleteProjectGroupRequest>,
    ) -> Result<pb::DeleteProjectGroupResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let id: ProjectGroupId = parse::required(request.into_inner().group_id, "group_id")?;

        // Check the route permission, then delete the group.
        let route = ProjectGroupHolder::DELETE_PROJECT_GROUP;
        self.authorize_group(&caller, route, id).await?;
        service::delete(&self.pool, &self.hooks, caller.actor, id).await?;
        Ok(pb::DeleteProjectGroupResponse {})
    }

    /// Handles `MoveProject`: a new group and a new place for a project. An unknown project gives
    /// `PERMISSION_DENIED`.
    async fn move_to(
        &self,
        request: Request<pb::MoveProjectRequest>,
    ) -> Result<pb::MoveProjectResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let project_id: ProjectId = parse::required(request.project_id, "project_id")?;
        let moved = ProjectMove {
            project_id,
            group_id: request
                .group_id
                .map(|id| parse::field(id, "group_id"))
                .transpose()?,
            before: request
                .before_project_id
                .map(|id| parse::field(id, "before_project_id"))
                .transpose()?,
        };

        // Check the route permission. An unknown project gets the answer of a refusal.
        let check = ProjectGroupAccess::MOVE_PROJECT.on(project_id);
        let Some(workspace) = service::project_workspace(&self.pool, project_id).await? else {
            return Err(AccessError::denied(&caller, &check).into());
        };
        self.gate
            .authorize(&caller, check.within(workspace))
            .await?;

        // Move the project.
        let project = service::move_project(&self.pool, moved).await?;
        Ok(pb::MoveProjectResponse {
            project: Some(project.to_proto()),
        })
    }

    /// Checks a route on the workspace of a group. An unknown group and a refusal get the same
    /// answer, which names the group, not its workspace.
    async fn authorize_group(
        &self,
        caller: &Principal,
        route: Permission<Workspace>,
        id: ProjectGroupId,
    ) -> Result<(), Error> {
        // Refuse an unknown group.
        let object = format!("project_group:{id}");
        let name = route.name();
        let reason = format!("{name} on {object} is denied");
        let Some(workspace) = service::workspace(&self.pool, id).await? else {
            return Err(AccessError::refused(caller, name, &object, reason).into());
        };

        // Check the workspace, and hide it in a refusal.
        match self.gate.authorize(caller, route.on(workspace)).await {
            Ok(()) => Ok(()),
            Err(AccessError::Denied(_)) => Err(AccessError::Denied(reason).into()),
            Err(error) => Err(error.into()),
        }
    }
}

impl ToProto<pb::ProjectGroup> for ProjectGroup {
    fn to_proto(self) -> pb::ProjectGroup {
        pb::ProjectGroup {
            id: Some(self.id.to_proto()),
            workspace_id: Some(self.workspace_id.to_proto()),
            created_by: Some(self.created_by.to_proto()),
            name: self.name,
            created_at: Some(parse::timestamp(self.created_at)),
        }
    }
}
