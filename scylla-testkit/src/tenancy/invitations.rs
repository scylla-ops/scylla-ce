//! The `TestApp` helpers of `InvitationService`.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::workspaces::v1::invitation_service_client::InvitationServiceClient;
use scylla_proto::workspaces::v1::{
    AcceptInvitationRequest, CreateInvitationRequest, Invitation, ListInvitationsRequest,
    ResendInvitationRequest, RevokeInvitationRequest,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::rpc::page;
use crate::{id, text, with_token, TestApp};

impl TestApp {
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

    /// Calls `ListInvitations` as `actor` for one page of 100, and returns the invitations in
    /// list order. The id is text, so a test can send a bad id.
    pub async fn list_invitations(
        &self,
        actor: ActorId,
        workspace: &str,
    ) -> Result<Vec<Invitation>, Status> {
        let request = ListInvitationsRequest {
            workspace_id: Some(id(workspace)),
            page: Some(page(100, "")),
        };
        let request = self.authed(actor, request).await;
        let response = self.invitations().list_invitations(request).await?;
        Ok(response.into_inner().invitations)
    }

    /// Calls `RevokeInvitation` as `actor`. The ids are text, so a test can send a bad id.
    pub async fn revoke_invitation(
        &self,
        actor: ActorId,
        workspace: &str,
        invitation: &str,
    ) -> Result<(), Status> {
        let request = RevokeInvitationRequest {
            workspace_id: Some(id(workspace)),
            invitation_id: Some(id(invitation)),
        };
        let request = self.authed(actor, request).await;
        self.invitations().revoke_invitation(request).await?;
        Ok(())
    }

    /// Calls `ResendInvitation` as `actor`. The ids are text, so a test can send a bad id.
    pub async fn resend_invitation(
        &self,
        actor: ActorId,
        workspace: &str,
        invitation: &str,
    ) -> Result<Invitation, Status> {
        let request = ResendInvitationRequest {
            workspace_id: Some(id(workspace)),
            invitation_id: Some(id(invitation)),
        };
        let request = self.authed(actor, request).await;
        let response = self.invitations().resend_invitation(request).await?;
        Ok(response
            .into_inner()
            .invitation
            .expect("invitation in response"))
    }

    /// Returns a client of `InvitationService`.
    fn invitations(&self) -> InvitationServiceClient<Channel> {
        InvitationServiceClient::new(self.channel.clone())
    }
}
