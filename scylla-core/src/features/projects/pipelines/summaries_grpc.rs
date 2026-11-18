//! The RPC of `PipelineService` for the dashboard: the summaries of the pipelines that the caller
//! can see in a workspace.

use scylla_domain::projects::pipelines::{
    ListSummaries, PipelineAccess, PipelineCaller, PipelineHolder, DEFAULT_RUNS, MAX_RUNS,
};
use scylla_domain::projects::Visible;
use scylla_domain::tenancy::catalog;
use scylla_extension::AccessError;
use scylla_proto::pipelines::v1 as pb;
use tonic::{Request, Response, Status};

use super::grpc::PipelinesGrpc;
use super::{summaries_convert, summaries_service};
use crate::api::parse;
use crate::Error;

impl PipelinesGrpc {
    /// Answers `ListPipelineSummaries`: one page of the pipelines of a workspace, in the
    /// projects that the caller can see, with their recent jobs. A filter causes no refusal.
    pub(super) async fn list_summaries(
        &self,
        request: Request<pb::ListPipelineSummariesRequest>,
    ) -> Result<Response<pb::ListPipelineSummariesResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let query = ListSummaries {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            project_ids: summaries_convert::ids(request.project_ids, "project_ids")?,
            pipeline_ids: summaries_convert::ids(request.pipeline_ids, "pipeline_ids")?,
            runs: runs(request.runs)?,
            order: summaries_convert::order(request.order)?,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission on the caller: the list shows only what it can see.
        let check = PipelineCaller::LIST_PIPELINE_SUMMARIES.on(caller.actor.id);
        self.gate
            .authorize(&caller, check)
            .await
            .map_err(Error::from)?;

        // Find the workspaces where the caller has a role, and the projects shared with it.
        let (workspaces, shared) = tokio::join!(
            self.gate
                .granted(&caller, PipelineHolder::LIST_PIPELINE_SUMMARIES),
            self.gate
                .granted(&caller, PipelineAccess::LIST_PIPELINE_SUMMARIES),
        );
        let all = self
            .gate
            .holds_on_instance(&caller, catalog::VIEW_JOBS)
            .await
            .map_err(Error::from)?;
        let visible = Visible {
            caller: caller.actor.id,
            workspaces: or_owned_only(workspaces),
            projects: or_owned_only(shared),
            all,
        };

        // Read one page of the summaries.
        let page = summaries_service::list(&self.pool, query, visible).await?;
        Ok(Response::new(summaries_convert::response(page)))
    }
}

/// Reads the number of recent jobs: 0 gives the default, and a number above the maximum gives
/// `INVALID_ARGUMENT`.
fn runs(raw: u32) -> Result<u32, Error> {
    match raw {
        0 => Ok(DEFAULT_RUNS),
        runs if runs > MAX_RUNS => Err(Error::invalid("runs", "must be at most 50")),
        runs => Ok(runs),
    }
}

/// Returns the ids of a lookup, or none after a failure: the list then shows the pipelines of
/// the owned workspaces only.
fn or_owned_only<T>(lookup: Result<Vec<T>, AccessError>) -> Vec<T> {
    lookup.unwrap_or_else(|error| {
        tracing::warn!(%error, "a lookup failed: the list shows only the owned workspaces");
        Vec::new()
    })
}
