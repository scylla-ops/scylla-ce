//! The `TestApp` helpers of `WorkspaceService`: the workspaces and their owners.

use scylla_domain::actors::ActorId;
use scylla_proto::common::v1 as common;
use scylla_proto::workspaces::v1::workspace_service_client::WorkspaceServiceClient;
use scylla_proto::workspaces::v1::{
    AddOwnerRequest, CreateWorkspaceRequest, GetWorkspaceRequest, ListOwnersRequest,
    ListWorkspacesRequest, RemoveOwnerRequest, Workspace,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::{id, text, TestApp};

impl TestApp {
    /// Calls `CreateWorkspace` as `actor`.
    pub async fn create_workspace(&self, actor: ActorId, name: &str) -> Result<Workspace, Status> {
        let request = CreateWorkspaceRequest {
            name: name.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().create_workspace(request).await?;
        Ok(response
            .into_inner()
            .workspace
            .expect("workspace in response"))
    }

    /// Calls `GetWorkspace` as `actor`. The id is text, so a test can send a bad id.
    pub async fn get_workspace(
        &self,
        actor: ActorId,
        workspace: &str,
    ) -> Result<Workspace, Status> {
        let request = GetWorkspaceRequest {
            workspace_id: Some(id(workspace)),
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().get_workspace(request).await?;
        Ok(response
            .into_inner()
            .workspace
            .expect("workspace in response"))
    }

    /// Calls `ListWorkspaces` as `actor` for one page of 100, and returns the names of the
    /// workspaces in list order.
    pub async fn visible_workspaces(&self, actor: ActorId) -> Result<Vec<String>, Status> {
        let request = ListWorkspacesRequest {
            page: Some(page_of_100()),
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().list_workspaces(request).await?;
        let workspaces = response.into_inner().workspaces;
        Ok(workspaces
            .into_iter()
            .map(|workspace| workspace.name)
            .collect())
    }

    /// Calls `AddOwner` as `actor`. The ids are text, so a test can send a bad id.
    pub async fn add_owner(
        &self,
        actor: ActorId,
        workspace: &str,
        user: &str,
    ) -> Result<(), Status> {
        let request = AddOwnerRequest {
            workspace_id: Some(id(workspace)),
            user_id: Some(id(user)),
        };
        let request = self.authed(actor, request).await;
        self.workspaces().add_owner(request).await?;
        Ok(())
    }

    /// Calls `RemoveOwner` as `actor`. The ids are text, so a test can send a bad id.
    pub async fn remove_owner(
        &self,
        actor: ActorId,
        workspace: &str,
        user: &str,
    ) -> Result<(), Status> {
        let request = RemoveOwnerRequest {
            workspace_id: Some(id(workspace)),
            user_id: Some(id(user)),
        };
        let request = self.authed(actor, request).await;
        self.workspaces().remove_owner(request).await?;
        Ok(())
    }

    /// Calls `ListOwners` as `actor` for one page of 100, and returns the user ids in list
    /// order.
    pub async fn owners(&self, actor: ActorId, workspace: &str) -> Result<Vec<String>, Status> {
        let request = ListOwnersRequest {
            workspace_id: Some(id(workspace)),
            page: Some(page_of_100()),
        };
        let request = self.authed(actor, request).await;
        let response = self.workspaces().list_owners(request).await?;
        let owners = response.into_inner().owners;
        Ok(owners
            .iter()
            .map(|owner| text(&owner.user_id).to_owned())
            .collect())
    }

    /// Returns a client of `WorkspaceService`.
    fn workspaces(&self) -> WorkspaceServiceClient<Channel> {
        WorkspaceServiceClient::new(self.channel.clone())
    }
}

/// The first page of 100 items.
fn page_of_100() -> common::PageRequest {
    common::PageRequest {
        size: 100,
        token: String::new(),
    }
}
