//! The `TestApp` helpers of `WorkspaceService`.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::common::v1::IconChange;
use scylla_proto::workspaces::v1::get_workspace_request::Workspace as Named;
use scylla_proto::workspaces::v1::workspace_service_client::WorkspaceServiceClient;
use scylla_proto::workspaces::v1::{
    CreateWorkspaceRequest, DeleteWorkspaceRequest, GetWorkspaceRequest, GetWorkspaceResponse,
    ListAllWorkspacesRequest, ListAllWorkspacesResponse, ListWorkspacesRequest,
    ListWorkspacesResponse, UpdateWorkspaceRequest, Workspace,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::rpc::page;
use crate::{id, TestApp};

impl TestApp {
    /// Calls `CreateWorkspace` as `actor`, with a slug from the name.
    pub async fn create_workspace(&self, actor: ActorId, name: &str) -> Result<Workspace, Status> {
        self.create_workspace_with(actor, name, None).await
    }

    /// Calls `CreateWorkspace` as `actor`, with a slug if one is given.
    pub async fn create_workspace_with(
        &self,
        actor: ActorId,
        name: &str,
        slug: Option<&str>,
    ) -> Result<Workspace, Status> {
        // Raise the limit of the workspaces with no route: most tests do not test it.
        sqlx::query("UPDATE instance_settings SET workspace_limit = 1000")
            .execute(&self.pool)
            .await
            .expect("raise the limit of the workspaces");
        self.create_workspace_as_is(actor, name, slug).await
    }

    /// Calls `CreateWorkspace` as `actor`, with the limit of the instance.
    pub async fn create_workspace_as_is(
        &self,
        actor: ActorId,
        name: &str,
        slug: Option<&str>,
    ) -> Result<Workspace, Status> {
        let request = CreateWorkspaceRequest {
            name: name.to_owned(),
            slug: slug.map(str::to_owned),
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().create_workspace(request).await?;
        Ok(response
            .into_inner()
            .workspace
            .expect("workspace in response"))
    }

    /// Calls `GetWorkspace` as `actor` with an id. The id is text, so a test can send a bad id.
    pub async fn get_workspace(
        &self,
        actor: ActorId,
        workspace: &str,
    ) -> Result<Workspace, Status> {
        self.get_named(actor, Named::WorkspaceId(id(workspace)))
            .await
    }

    /// Calls `GetWorkspace` as `actor` with a slug.
    pub async fn get_workspace_by_slug(
        &self,
        actor: ActorId,
        slug: &str,
    ) -> Result<Workspace, Status> {
        self.get_named(actor, Named::Slug(slug.to_owned())).await
    }

    /// Calls `UpdateWorkspace` as `actor`. A field that is `None` does not change.
    pub async fn update_workspace(
        &self,
        actor: ActorId,
        workspace: &str,
        name: Option<&str>,
        slug: Option<&str>,
    ) -> Result<Workspace, Status> {
        let request = UpdateWorkspaceRequest {
            workspace_id: Some(id(workspace)),
            name: name.map(str::to_owned),
            slug: slug.map(str::to_owned),
            personal_tokens_enabled: None,
            time_zone: None,
            icon: None,
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().update_workspace(request).await?;
        Ok(response
            .into_inner()
            .workspace
            .expect("workspace in response"))
    }

    /// Calls `UpdateWorkspace` as `actor` with the time zone only.
    pub async fn set_workspace_time_zone(
        &self,
        actor: ActorId,
        workspace: &str,
        time_zone: &str,
    ) -> Result<Workspace, Status> {
        let request = UpdateWorkspaceRequest {
            workspace_id: Some(id(workspace)),
            name: None,
            slug: None,
            personal_tokens_enabled: None,
            time_zone: Some(time_zone.to_owned()),
            icon: None,
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().update_workspace(request).await?;
        Ok(response
            .into_inner()
            .workspace
            .expect("workspace in response"))
    }

    /// Calls `UpdateWorkspace` as `actor` with the icon only.
    pub async fn set_workspace_icon(
        &self,
        actor: ActorId,
        workspace: &str,
        icon: IconChange,
    ) -> Result<Workspace, Status> {
        let request = UpdateWorkspaceRequest {
            workspace_id: Some(id(workspace)),
            name: None,
            slug: None,
            personal_tokens_enabled: None,
            time_zone: None,
            icon: Some(icon),
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().update_workspace(request).await?;
        Ok(response
            .into_inner()
            .workspace
            .expect("workspace in response"))
    }

    /// Calls `DeleteWorkspace` as `actor`. The id is text, so a test can send a bad id.
    pub async fn delete_workspace(&self, actor: ActorId, workspace: &str) -> Result<(), Status> {
        let request = DeleteWorkspaceRequest {
            workspace_id: Some(id(workspace)),
        };
        let request = self.authed(actor, request).await;
        self.workspaces().delete_workspace(request).await?;
        Ok(())
    }

    /// Calls `ListWorkspaces` as `actor` for one page of 100, and returns the names of the
    /// workspaces in list order.
    pub async fn visible_workspaces(&self, actor: ActorId) -> Result<Vec<String>, Status> {
        let workspaces = self.list_workspaces(actor, false).await?.workspaces;
        Ok(workspaces
            .into_iter()
            .map(|workspace| workspace.name)
            .collect())
    }

    /// Calls `ListWorkspaces` as `actor` for one page of 100. `only_with_role` keeps only the
    /// workspaces where the actor has a role.
    pub async fn list_workspaces(
        &self,
        actor: ActorId,
        only_with_role: bool,
    ) -> Result<ListWorkspacesResponse, Status> {
        let request = ListWorkspacesRequest {
            page: Some(page(100, "")),
            only_with_role,
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().list_workspaces(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `ListAllWorkspaces` as `actor`.
    pub async fn list_all_workspaces(
        &self,
        actor: ActorId,
        request: ListAllWorkspacesRequest,
    ) -> Result<ListAllWorkspacesResponse, Status> {
        let request = self.authed(actor, request).await;
        let response = self.workspaces().list_all_workspaces(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `GetWorkspace` as `actor` with an id, and returns the whole answer.
    pub async fn get_workspace_with_details(
        &self,
        actor: ActorId,
        workspace: &str,
    ) -> Result<GetWorkspaceResponse, Status> {
        let request = GetWorkspaceRequest {
            workspace: Some(Named::WorkspaceId(id(workspace))),
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().get_workspace(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `GetWorkspace` as `actor` with an id or a slug.
    async fn get_named(&self, actor: ActorId, named: Named) -> Result<Workspace, Status> {
        let request = GetWorkspaceRequest {
            workspace: Some(named),
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().get_workspace(request).await?;
        Ok(response
            .into_inner()
            .workspace
            .expect("workspace in response"))
    }

    /// Returns a client of `WorkspaceService`.
    fn workspaces(&self) -> WorkspaceServiceClient<Channel> {
        WorkspaceServiceClient::new(self.channel.clone())
    }
}
