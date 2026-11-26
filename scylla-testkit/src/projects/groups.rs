//! The `TestApp` helpers of `ProjectGroupService`. The ids are text, so a test can send a bad id.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::projects::v1::project_group_service_client::ProjectGroupServiceClient;
use scylla_proto::projects::v1::{
    CreateProjectGroupRequest, DeleteProjectGroupRequest, ListProjectGroupsRequest,
    MoveProjectRequest, Project, ProjectGroup, UpdateProjectGroupRequest,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::rpc::page;
use crate::{id, TestApp};

impl TestApp {
    /// Returns a client of `ProjectGroupService`.
    fn project_groups(&self) -> ProjectGroupServiceClient<Channel> {
        ProjectGroupServiceClient::new(self.channel.clone())
    }

    /// Calls `ListProjectGroups` as `actor` for one page of 100, and returns the groups.
    pub async fn list_project_groups(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
    ) -> Result<Vec<ProjectGroup>, Status> {
        let request = ListProjectGroupsRequest {
            workspace_id: Some(id(workspace.to_string())),
            page: Some(page(100, "")),
        };
        let request = self.authed(actor, request).await;
        let response = self.project_groups().list_project_groups(request).await?;
        Ok(response.into_inner().groups)
    }

    /// Calls `CreateProjectGroup` as `actor`, in `workspace`.
    pub async fn create_project_group(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        name: &str,
    ) -> Result<ProjectGroup, Status> {
        let request = CreateProjectGroupRequest {
            workspace_id: Some(id(workspace.to_string())),
            name: name.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.project_groups().create_project_group(request).await?;
        Ok(response.into_inner().group.expect("group in response"))
    }

    /// Calls `UpdateProjectGroup` as `actor`. A field that is `None` does not change.
    pub async fn update_project_group(
        &self,
        actor: ActorId,
        group: &str,
        name: Option<&str>,
        position: Option<u32>,
    ) -> Result<ProjectGroup, Status> {
        let request = UpdateProjectGroupRequest {
            group_id: Some(id(group)),
            name: name.map(str::to_owned),
            position,
        };
        let request = self.authed(actor, request).await;
        let response = self.project_groups().update_project_group(request).await?;
        Ok(response.into_inner().group.expect("group in response"))
    }

    /// Calls `DeleteProjectGroup` as `actor`.
    pub async fn delete_project_group(&self, actor: ActorId, group: &str) -> Result<(), Status> {
        let request = DeleteProjectGroupRequest {
            group_id: Some(id(group)),
        };
        let request = self.authed(actor, request).await;
        self.project_groups().delete_project_group(request).await?;
        Ok(())
    }

    /// Calls `MoveProject` as `actor`: to `group`, or to no group, before `before`, or last.
    pub async fn move_project(
        &self,
        actor: ActorId,
        project: &str,
        group: Option<&str>,
        before: Option<&str>,
    ) -> Result<Project, Status> {
        let request = MoveProjectRequest {
            project_id: Some(id(project)),
            group_id: group.map(id),
            before_project_id: before.map(id),
        };
        let request = self.authed(actor, request).await;
        let response = self.project_groups().move_project(request).await?;
        Ok(response.into_inner().project.expect("project in response"))
    }
}
