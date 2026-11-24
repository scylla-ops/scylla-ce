//! The gRPC routes of the open invitations of a workspace: `ListInvitations`,
//! `RevokeInvitation` and `ResendInvitation`.

use scylla_domain::tenancy::invitations::{InvitationId, ListInvitations};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_proto::workspaces::v1 as pb;
use tonic::Request;

use super::grpc::InvitationsGrpc;
use super::service;
use crate::api::parse::{self, ToProto};
use crate::Error;

impl InvitationsGrpc {
    /// Handles `ListInvitations`: the open invitations of a workspace, for its admins.
    pub(super) async fn list(
        &self,
        request: Request<pb::ListInvitationsRequest>,
    ) -> Result<pb::ListInvitationsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let query = ListInvitations {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission on the workspace, then read one page.
        self.ctx
            .gate
            .authorize(&caller, Workspace::LIST_INVITATIONS.on(query.workspace_id))
            .await?;
        let page = service::list(&self.ctx.pool, query).await?;
        Ok(pb::ListInvitationsResponse {
            invitations: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `RevokeInvitation`: the link of an open invitation stops.
    pub(super) async fn revoke(
        &self,
        request: Request<pb::RevokeInvitationRequest>,
    ) -> Result<pb::RevokeInvitationResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let id: InvitationId = parse::required(request.invitation_id, "invitation_id")?;

        // Check the route permission on the workspace, then delete the invitation.
        self.ctx
            .gate
            .authorize(&caller, Workspace::REVOKE_INVITATION.on(workspace))
            .await?;
        service::revoke(&self.ctx.pool, workspace, id).await?;
        Ok(pb::RevokeInvitationResponse {})
    }

    /// Handles `ResendInvitation`: a new link for an open invitation, at most one each minute.
    pub(super) async fn resend(
        &self,
        request: Request<pb::ResendInvitationRequest>,
    ) -> Result<pb::ResendInvitationResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let id: InvitationId = parse::required(request.invitation_id, "invitation_id")?;

        // Check the route permission on the workspace, then send a new link.
        self.ctx
            .gate
            .authorize(&caller, Workspace::RESEND_INVITATION.on(workspace))
            .await?;
        let invitation = service::resend(&self.ctx, workspace, id).await?;
        Ok(pb::ResendInvitationResponse {
            invitation: Some(invitation.to_proto()),
        })
    }
}
