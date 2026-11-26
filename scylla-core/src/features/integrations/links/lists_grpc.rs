//! The RPC of `RepositoryService` for the lists: the links of the projects of a workspace that
//! the caller can read, in one call.

use scylla_domain::integrations::links::{RepositoryAccess, RepositoryCaller, RepositoryHolder};
use scylla_domain::projects::{ProjectId, Visible};
use scylla_domain::tenancy::catalog;
use scylla_extension::AccessError;
use scylla_proto::integrations::v1 as pb;
use tonic::Request;

use super::grpc::LinksGrpc;
use super::service;
use crate::api::parse::{self, ToProto};
use crate::Error;

/// The largest number of project ids in the filter of a list.
const MAX_PROJECT_IDS: usize = 100;

impl LinksGrpc {
    /// Handles `ListRepositoryLinks`: the links of the projects of a workspace that the caller can
    /// read. A filter causes no refusal.
    pub(super) async fn list(
        &self,
        request: Request<pb::ListRepositoryLinksRequest>,
    ) -> Result<pb::ListRepositoryLinksResponse, Error> {
        // Authenticate the caller, then read the request.
        let gate = &self.ctx.gate;
        let caller = gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace = parse::required(request.workspace_id, "workspace_id")?;
        if request.project_ids.len() > MAX_PROJECT_IDS {
            return Err(Error::invalid("project_ids", "must have at most 100 ids"));
        }
        let projects = request
            .project_ids
            .into_iter()
            .map(|id| parse::field::<_, ProjectId>(id, "project_ids"))
            .collect::<Result<Vec<_>, _>>()?;

        // Check the route permission on the caller: the list shows only what it can read.
        let check = RepositoryCaller::LIST_REPOSITORY_LINKS.on(caller.actor.id);
        gate.authorize(&caller, check).await?;

        // Find the workspaces where the caller has a role, and the projects shared with it.
        let (workspaces, shared) = tokio::join!(
            gate.granted(&caller, RepositoryHolder::LIST_REPOSITORY_LINKS),
            gate.granted(&caller, RepositoryAccess::LIST_REPOSITORY_LINKS),
        );
        let all = gate
            .holds_on_instance(&caller, catalog::VIEW_REPOSITORY)
            .await?;
        let visible = Visible {
            caller: caller.actor.id,
            workspaces: or_owned_only(workspaces),
            projects: or_owned_only(shared),
            all,
        };

        // Read the links.
        let links = service::list(&self.ctx, workspace, &projects, &visible).await?;
        Ok(pb::ListRepositoryLinksResponse {
            links: links.into_iter().map(ToProto::to_proto).collect(),
        })
    }
}

/// Returns the ids of a lookup, or none after a failure: the list then shows the links of the
/// owned workspaces only.
fn or_owned_only<T>(lookup: Result<Vec<T>, AccessError>) -> Vec<T> {
    lookup.unwrap_or_else(|error| {
        tracing::warn!(%error, "a lookup failed: the list shows only the owned workspaces");
        Vec::new()
    })
}
