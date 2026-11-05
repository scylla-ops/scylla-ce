//! The gRPC routes of the owners of a workspace: `AddOwner`, `RemoveOwner` and `ListOwners`.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::owners::ListOwners;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_proto::workspaces::v1 as pb;
use tonic::Request;

use super::grpc::WorkspacesGrpc;
use super::service;
use crate::api::parse::{self, ToProto};
use crate::features::tenancy::grants::membership::{end_membership, Leaving};
use crate::Error;

impl WorkspacesGrpc {
    /// Handles `AddOwner`. Only an owner passes: no role gives this permission. An unknown
    /// workspace gets the answer of a refusal.
    pub(super) async fn add(
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
        service::add_owner(&self.pool, &self.hooks, caller.actor, workspace, user).await?;
        Ok(pb::AddOwnerResponse {})
    }

    /// Handles `RemoveOwner`. Only an owner passes, and the last owner stays.
    pub(super) async fn remove(
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
        let leaving = Leaving {
            pool: &self.pool,
            gate: self.gate.as_ref(),
            grants: &self.grants,
            hooks: &self.hooks,
        };
        end_membership(&leaving, caller.actor, user, workspace).await?;
        Ok(pb::RemoveOwnerResponse {})
    }

    /// Handles `ListOwners`. A reader of the workspace passes.
    pub(super) async fn owners(
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
