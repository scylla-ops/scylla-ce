//! The gRPC routes of `WorkspaceService` that list workspaces, `ListWorkspaces` and
//! `ListAllWorkspaces`, and the details of the workspaces of an answer.

use scylla_domain::platform::Platform;
use scylla_domain::tenancy::catalog;
use scylla_domain::tenancy::workspaces::lists::ListAllWorkspaces;
use scylla_domain::tenancy::workspaces::{
    ListWorkspaces, VisibleWorkspaces, Workspace, WorkspaceAdmin, WorkspaceCaller, WorkspaceId,
};
use scylla_extension::Principal;
use scylla_proto::workspaces::v1 as pb;
use tonic::Request;

use super::grpc::WorkspacesGrpc;
use super::{convert, details, service};
use crate::api::parse::{self, ToProto};
use crate::Error;

impl WorkspacesGrpc {
    /// Returns the workspaces where the caller has a role. A failed lookup gives none.
    async fn granted(&self, caller: &Principal) -> Vec<WorkspaceId> {
        self.gate
            .granted(caller, Workspace::LIST_WORKSPACES)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "a lookup failed: the list shows only the owned workspaces");
                Vec::new()
            })
    }

    /// Returns `true` if an instance role shows each workspace to the caller.
    pub(super) async fn sees_all(&self, caller: &Principal) -> Result<bool, Error> {
        let all = self
            .gate
            .holds_on_instance(caller, catalog::VIEW_ALL_WORKSPACES)
            .await?;
        Ok(all)
    }

    /// Returns the details of workspaces for the caller, in the order of the ids. `with_email`
    /// gives the primary email of the owners.
    pub(super) async fn details_of(
        &self,
        caller: &Principal,
        ids: &[WorkspaceId],
        with_email: bool,
    ) -> Result<Vec<pb::WorkspaceDetails>, Error> {
        let gate = self.gate.as_ref();
        let found = details::details(&self.pool, gate, caller.actor.id, ids, with_email).await?;
        Ok(found.into_iter().map(ToProto::to_proto).collect())
    }

    /// Handles `ListWorkspaces`: the workspaces that the caller owns or where it has a role, and
    /// each workspace for a caller with an instance role that shows them, unless it asks only for
    /// its own.
    pub(super) async fn list(
        &self,
        request: Request<pb::ListWorkspacesRequest>,
    ) -> Result<pb::ListWorkspacesResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let query = ListWorkspaces {
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission on the caller: the list shows only what it can read.
        self.gate
            .authorize(
                &caller,
                WorkspaceCaller::LIST_WORKSPACES.on(caller.actor.id),
            )
            .await?;

        // Read one page of the workspaces that the caller can read.
        let sees_all = self.sees_all(&caller).await?;
        let visible = VisibleWorkspaces {
            caller: caller.actor.id,
            granted: self.granted(&caller).await,
            all: sees_all && !request.only_with_role,
        };
        let page = service::list(&self.pool, query, visible).await?;

        // Add the details of the workspaces of the page.
        let ids: Vec<WorkspaceId> = page.items.iter().map(|workspace| workspace.id).collect();
        let details = self.details_of(&caller, &ids, sees_all).await?;
        Ok(pb::ListWorkspacesResponse {
            workspaces: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
            details,
        })
    }

    /// Handles `ListAllWorkspaces`: one page of the console of the workspaces of the instance.
    pub(super) async fn list_all(
        &self,
        request: Request<pb::ListAllWorkspacesRequest>,
    ) -> Result<pb::ListAllWorkspacesResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let query = ListAllWorkspaces {
            search: parse::field(request.query, "query")?,
            order: convert::order(request.order)?,
            others_only: request.others_only,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission on the platform.
        self.gate
            .authorize(
                &caller,
                WorkspaceAdmin::LIST_ALL_WORKSPACES.on(Platform::ID),
            )
            .await?;

        // Read one page. Only the filter of the others needs the workspaces of the caller.
        let granted = match query.others_only {
            true => self.granted(&caller).await,
            false => Vec::new(),
        };
        let mine = VisibleWorkspaces {
            caller: caller.actor.id,
            granted,
            all: false,
        };
        let found = service::list_all(&self.pool, query, mine).await?;

        // Add the details of the workspaces of the page.
        let items = found.page.items;
        let ids: Vec<WorkspaceId> = items.iter().map(|workspace| workspace.id).collect();
        let details = self.details_of(&caller, &ids, true).await?;
        Ok(pb::ListAllWorkspacesResponse {
            workspaces: items.into_iter().map(ToProto::to_proto).collect(),
            details,
            page: Some(found.page.next_after.to_proto()),
            total: found.total,
        })
    }
}
