//! The job lists of `JobService`: the jobs of a pipeline, and the jobs of a workspace in the
//! projects that the caller can see.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{JobPage, JobScope, ListJobs, WorkspaceJobs, MAX_LIST_IDS};
use scylla_domain::projects::pipelines::{
    PipelineAccess, PipelineCaller, PipelineHolder, PipelineId,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Id;
use scylla_proto::common::v1::Id as ProtoId;
use scylla_proto::pipelines::v1 as pb;
use tonic::Request;

use super::grpc::JobsGrpc;
use super::{convert, service};
use crate::api::parse::{self, ToProto};
use crate::features::projects::pipelines::{access, service as pipelines};
use crate::Error;

impl JobsGrpc {
    /// Handles `ListPipelineJobs`: one page of the jobs of a pipeline, newest first.
    pub(super) async fn list_pipeline(
        &self,
        request: Request<pb::ListPipelineJobsRequest>,
    ) -> Result<pb::ListPipelineJobsResponse, Error> {
        // Authenticate the caller, read the request, and check the permission on the pipeline.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: PipelineId = parse::required(request.pipeline_id, "pipeline_id")?;
        let page = parse::optional(request.page, "page")?;
        let filter = parse::optional(request.filter, "filter")?;
        let place = pipelines::place(&self.pool, id).await?;
        let permission = PipelineAccess::LIST_PIPELINE_JOBS;
        let object = format!("pipeline:{id}");
        access::authorize(self.gate.as_ref(), &caller, permission, object, place).await?;

        // Read one page.
        let query = ListJobs {
            scope: JobScope::Pipeline(id),
            page,
            filter,
        };
        let page = service::list(&self.pool, query).await?;
        Ok(pb::ListPipelineJobsResponse {
            jobs: self.protos(page.items, false),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `ListWorkspaceJobs`: one page of the jobs of a workspace, newest first, in the
    /// projects that the caller can see. A filter never causes a refusal.
    pub(super) async fn list_workspace(
        &self,
        request: Request<pb::ListWorkspaceJobsRequest>,
    ) -> Result<pb::ListWorkspaceJobsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let summary = request.view() == pb::JobView::Summary;
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let agent: Option<AppId> = request
            .agent_id
            .map(|id| parse::field(id, "agent_id"))
            .transpose()?;
        let projects = ids(request.project_ids, "project_ids")?;
        let pipelines = ids(request.pipeline_ids, "pipeline_ids")?;
        let page = parse::optional(request.page, "page")?;
        let filter = parse::optional(request.filter, "filter")?;

        // Check the route permission of the caller, then find the projects that it can see.
        let check = PipelineCaller::LIST_WORKSPACE_JOBS.on(caller.actor.id);
        self.gate
            .authorize(&caller, check.within(workspace))
            .await?;
        let visible = access::visible_projects(
            self.gate.as_ref(),
            &caller,
            workspace,
            PipelineHolder::LIST_WORKSPACE_JOBS,
            PipelineAccess::LIST_WORKSPACE_JOBS,
        )
        .await;

        // Read one page of the jobs of the visible projects.
        let scope = WorkspaceJobs {
            workspace,
            visible,
            agent,
            projects,
            pipelines,
        };
        let query = ListJobs {
            scope: JobScope::Workspace(scope),
            page,
            filter,
        };
        let page: JobPage = service::list(&self.pool, query).await?;
        Ok(pb::ListWorkspaceJobsResponse {
            jobs: self.protos(page.items, summary),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Returns the protos of jobs, with no nodes for a summary.
    fn protos(&self, jobs: Vec<scylla_domain::projects::jobs::Job>, summary: bool) -> Vec<pb::Job> {
        jobs.into_iter()
            .map(|job| {
                let mut proto = convert::job_of(job, &self.registry);
                if summary {
                    proto.nodes.clear();
                }
                proto
            })
            .collect()
    }
}

/// Reads a list of ids of a request, at most `MAX_LIST_IDS`.
fn ids<T: ?Sized>(values: Vec<ProtoId>, field: &'static str) -> Result<Vec<Id<T>>, Error> {
    if values.len() > MAX_LIST_IDS {
        return Err(Error::invalid(
            field,
            format!("must have at most {MAX_LIST_IDS} ids"),
        ));
    }
    values
        .into_iter()
        .map(|value| parse::field(value, field))
        .collect()
}
