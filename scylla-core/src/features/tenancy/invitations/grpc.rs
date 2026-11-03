//! The gRPC handler of `InvitationService`: an invitation and its acceptance. The routes of the
//! open invitations are in `manage.rs`.

use scylla_domain::identity::users::{Email, KIND};
use scylla_domain::tenancy::invitations::Invitation;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceCaller, WorkspaceId};
use scylla_extension::AccessError;
use scylla_proto::workspaces::v1 as pb;
use scylla_proto::workspaces::v1::invitation_service_server::{
    InvitationService, InvitationServiceServer,
};
use tonic::{Request, Response, Status};

use super::service::{self, Pick};
use crate::api::parse::{self, ToProto};
use crate::features::identity::accounts::AccountContext;
use crate::Error;

/// Makes the service.
pub(crate) fn server(ctx: AccountContext) -> InvitationServiceServer<InvitationsGrpc> {
    InvitationServiceServer::new(InvitationsGrpc { ctx })
}

/// The handler.
pub(crate) struct InvitationsGrpc {
    /// What the account routes share.
    pub(super) ctx: AccountContext,
}

#[tonic::async_trait]
impl InvitationService for InvitationsGrpc {
    async fn create_invitation(
        &self,
        request: Request<pb::CreateInvitationRequest>,
    ) -> Result<Response<pb::CreateInvitationResponse>, Status> {
        Ok(Response::new(self.create(request).await?))
    }

    async fn accept_invitation(
        &self,
        request: Request<pb::AcceptInvitationRequest>,
    ) -> Result<Response<pb::AcceptInvitationResponse>, Status> {
        Ok(Response::new(self.accept(request).await?))
    }

    async fn list_invitations(
        &self,
        request: Request<pb::ListInvitationsRequest>,
    ) -> Result<Response<pb::ListInvitationsResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn revoke_invitation(
        &self,
        request: Request<pb::RevokeInvitationRequest>,
    ) -> Result<Response<pb::RevokeInvitationResponse>, Status> {
        Ok(Response::new(self.revoke(request).await?))
    }

    async fn resend_invitation(
        &self,
        request: Request<pb::ResendInvitationRequest>,
    ) -> Result<Response<pb::ResendInvitationResponse>, Status> {
        Ok(Response::new(self.resend(request).await?))
    }
}

impl InvitationsGrpc {
    /// Handles `CreateInvitation`: an admin of a workspace invites an email.
    async fn create(
        &self,
        request: Request<pb::CreateInvitationRequest>,
    ) -> Result<pb::CreateInvitationResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let email: Email = parse::field(request.email, "email")?;

        // Check the route permission on the workspace, then invite.
        self.ctx
            .gate
            .authorize(&caller, Workspace::CREATE_INVITATION.on(workspace))
            .await?;
        let invitation = service::create(
            &self.ctx,
            caller.actor.clone(),
            workspace,
            email,
            &request.role,
        )
        .await?;
        Ok(pb::CreateInvitationResponse {
            invitation: Some(invitation.to_proto()),
        })
    }

    /// Handles `AcceptInvitation`: a user with the verified email of the invitation joins the
    /// workspace with its role.
    async fn accept(
        &self,
        request: Request<pb::AcceptInvitationRequest>,
    ) -> Result<pb::AcceptInvitationResponse, Error> {
        // Authenticate the caller and check the route permission on the caller.
        let caller = self.ctx.gate.caller(&request).await?;
        let check = WorkspaceCaller::ACCEPT_INVITATION.on(caller.actor.id);
        self.ctx.gate.authorize(&caller, check.clone()).await?;
        if caller.actor.kind != KIND {
            let reason = "only a user accepts an invitation".to_owned();
            let object = check.object();
            return Err(AccessError::refused(&caller, check.permission(), object, reason).into());
        }

        // Accept the open invitation of the link, for a verified email of the caller.
        let token = request.into_inner().token;
        let workspace = service::accept(&self.ctx, Pick::Token(&token), caller.actor.id).await?;
        Ok(pb::AcceptInvitationResponse {
            workspace_id: Some(workspace.to_proto()),
        })
    }
}

impl ToProto<pb::Invitation> for Invitation {
    fn to_proto(self) -> pb::Invitation {
        pb::Invitation {
            id: Some(self.id.to_proto()),
            workspace_id: Some(self.workspace_id.to_proto()),
            email: self.email,
            role: self.role,
            invited_by: Some(self.invited_by.to_proto()),
            expires_at: Some(parse::timestamp(self.expires_at)),
            sent_at: Some(parse::timestamp(self.sent_at)),
        }
    }
}
