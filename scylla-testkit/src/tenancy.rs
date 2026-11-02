//! The `TestApp` helpers of the tenancy RPCs: the workspaces and their owners, the roles of
//! `AccessService`, and the invitations.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::access::v1::access_service_client::AccessServiceClient;
use scylla_proto::access::v1::{GrantRequest, ObjectRef, RevokeRequest};
use scylla_proto::common::v1 as common;
use scylla_proto::workspaces::v1::invitation_service_client::InvitationServiceClient;
use scylla_proto::workspaces::v1::workspace_service_client::WorkspaceServiceClient;
use scylla_proto::workspaces::v1::{
    AcceptInvitationRequest, AddOwnerRequest, CreateInvitationRequest, CreateWorkspaceRequest,
    GetWorkspaceRequest, Invitation, ListOwnersRequest, ListWorkspacesRequest, RemoveOwnerRequest,
    Workspace,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::{id, text, with_token, TestApp};

/// A role on an object, as text, so a test can send a bad value: `Role::on("project", id,
/// "viewer")`.
#[derive(Clone, Copy, Debug)]
pub struct Role<'a> {
    /// The kind of the object.
    pub kind: &'a str,
    /// The id of the object.
    pub id: &'a str,
    /// The name of the role.
    pub name: &'a str,
}

impl<'a> Role<'a> {
    /// Names a role on an object.
    pub fn on(kind: &'a str, id: &'a str, name: &'a str) -> Self {
        Self { kind, id, name }
    }

    /// Makes the object message.
    fn object(&self) -> Option<ObjectRef> {
        Some(ObjectRef {
            kind: self.kind.to_owned(),
            id: Some(id(self.id)),
        })
    }
}

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

    /// Calls `Grant` as `actor`: gives the role to the actor `to`, as text.
    pub async fn grant_role(&self, actor: ActorId, role: Role<'_>, to: &str) -> Result<(), Status> {
        let request = GrantRequest {
            object: role.object(),
            role: role.name.to_owned(),
            actor_id: Some(id(to)),
        };
        let request = self.authed(actor, request).await;
        AccessServiceClient::new(self.channel.clone())
            .grant(request)
            .await?;
        Ok(())
    }

    /// Calls `Revoke` as `actor`: takes back the role from the actor `from`, as text.
    pub async fn revoke_role(
        &self,
        actor: ActorId,
        role: Role<'_>,
        from: &str,
    ) -> Result<(), Status> {
        let request = RevokeRequest {
            object: role.object(),
            role: role.name.to_owned(),
            actor_id: Some(id(from)),
        };
        let request = self.authed(actor, request).await;
        AccessServiceClient::new(self.channel.clone())
            .revoke(request)
            .await?;
        Ok(())
    }

    /// Calls `CreateInvitation` as `actor`.
    pub async fn create_invitation(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        email: &str,
        role: &str,
    ) -> Result<Invitation, Status> {
        let request = CreateInvitationRequest {
            workspace_id: Some(id(workspace.to_string())),
            email: email.to_owned(),
            role: role.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.invitations().create_invitation(request).await?;
        Ok(response
            .into_inner()
            .invitation
            .expect("invitation in response"))
    }

    /// Calls `AcceptInvitation` with an access token, and returns the id of the workspace.
    pub async fn accept_invitation(&self, token: &str, invitation: &str) -> Result<String, Status> {
        let request = AcceptInvitationRequest {
            token: invitation.to_owned(),
        };
        let response = self
            .invitations()
            .accept_invitation(with_token(token, request))
            .await?;
        Ok(text(&response.into_inner().workspace_id).to_owned())
    }

    /// Returns a client of `WorkspaceService`.
    fn workspaces(&self) -> WorkspaceServiceClient<Channel> {
        WorkspaceServiceClient::new(self.channel.clone())
    }

    /// Returns a client of `InvitationService`.
    fn invitations(&self) -> InvitationServiceClient<Channel> {
        InvitationServiceClient::new(self.channel.clone())
    }
}

/// The first page of 100 items.
fn page_of_100() -> common::PageRequest {
    common::PageRequest {
        size: 100,
        token: String::new(),
    }
}
