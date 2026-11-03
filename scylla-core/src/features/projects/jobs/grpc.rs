//! The gRPC handler of `JobService`. The tail of the logs is in `tail.rs`.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{JobId, JobScope, ListJobs};
use scylla_domain::projects::pipelines::{PipelineAccess, PipelineHolder, PipelineId};
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Gate, Hooks, Principal};
use scylla_proto::pipelines::v1 as pb;
use scylla_proto::pipelines::v1::job_service_server::{JobService, JobServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::{service, tail};
use crate::api::parse::{self, ToProto};
use crate::features::identity::agents::Registry;
use crate::features::projects::pipelines::{access, service as pipelines};
use crate::Error;

/// Makes the service.
pub(crate) fn server(
    pool: PgPool,
    hooks: Arc<Hooks>,
    gate: Arc<dyn Gate>,
    registry: Arc<Registry>,
) -> JobServiceServer<JobsGrpc> {
    JobServiceServer::new(JobsGrpc {
        pool,
        hooks,
        gate,
        registry,
    })
}

/// The handler.
pub(crate) struct JobsGrpc {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The hooks of the edition.
    hooks: Arc<Hooks>,
    /// The access gate.
    pub(super) gate: Arc<dyn Gate>,
    /// The open streams of the agents.
    registry: Arc<Registry>,
}

#[tonic::async_trait]
impl JobService for JobsGrpc {
    type TailJobLogsStream = tail::Tail;

    async fn get_job(
        &self,
        request: Request<pb::GetJobRequest>,
    ) -> Result<Response<pb::GetJobResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let id = self
            .job_of(
                &caller,
                request.into_inner().job_id,
                PipelineAccess::GET_JOB,
            )
            .await?;
        let job = service::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound("job"))?;
        Ok(Response::new(pb::GetJobResponse {
            job: Some(job.to_proto()),
        }))
    }

    async fn cancel_job(
        &self,
        request: Request<pb::CancelJobRequest>,
    ) -> Result<Response<pb::CancelJobResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let id = self
            .job_of(
                &caller,
                request.into_inner().job_id,
                PipelineAccess::CANCEL_JOB,
            )
            .await?;
        let job = service::cancel(&self.pool, &self.registry, id).await?;
        Ok(Response::new(pb::CancelJobResponse {
            job: Some(job.to_proto()),
        }))
    }

    async fn delete_job(
        &self,
        request: Request<pb::DeleteJobRequest>,
    ) -> Result<Response<pb::DeleteJobResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let id = self
            .job_of(
                &caller,
                request.into_inner().job_id,
                PipelineAccess::DELETE_JOB,
            )
            .await?;
        let actor = caller.actor;
        service::delete(&self.pool, &self.hooks, &self.registry, actor, id).await?;
        Ok(Response::new(pb::DeleteJobResponse {}))
    }

    async fn rerun_job(
        &self,
        request: Request<pb::RerunJobRequest>,
    ) -> Result<Response<pb::RerunJobResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let id = self
            .job_of(
                &caller,
                request.into_inner().job_id,
                PipelineAccess::RERUN_JOB,
            )
            .await?;
        let actor = caller.actor;
        let job = service::rerun(&self.pool, &self.hooks, &self.registry, actor, id).await?;
        Ok(Response::new(pb::RerunJobResponse {
            job: Some(job.to_proto()),
        }))
    }

    async fn list_pipeline_jobs(
        &self,
        request: Request<pb::ListPipelineJobsRequest>,
    ) -> Result<Response<pb::ListPipelineJobsResponse>, Status> {
        // Authenticate the caller, read the request, and check the permission on the pipeline.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let id: PipelineId = parse::required(request.pipeline_id, "pipeline_id")?;
        let page = parse::optional(request.page, "page")?;
        let place = pipelines::place(&self.pool, id).await?;
        let permission = PipelineAccess::LIST_PIPELINE_JOBS;
        access::authorize(
            self.gate.as_ref(),
            &caller,
            permission,
            format!("pipeline:{id}"),
            place,
        )
        .await?;

        // Read one page.
        let query = ListJobs {
            scope: JobScope::Pipeline(id),
            page,
        };
        let page = service::list(&self.pool, query).await?;
        Ok(Response::new(pb::ListPipelineJobsResponse {
            jobs: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        }))
    }

    async fn list_workspace_jobs(
        &self,
        request: Request<pb::ListWorkspaceJobsRequest>,
    ) -> Result<Response<pb::ListWorkspaceJobsResponse>, Status> {
        // Authenticate the caller, read the request, and check the permission on the workspace.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let agent: Option<AppId> = request
            .agent_id
            .map(|id| parse::field(id, "agent_id"))
            .transpose()?;
        let page = parse::optional(request.page, "page")?;
        self.gate
            .authorize(&caller, PipelineHolder::LIST_WORKSPACE_JOBS.on(workspace))
            .await
            .map_err(Error::from)?;

        // Read one page.
        let query = ListJobs {
            scope: JobScope::Workspace(workspace, agent),
            page,
        };
        let page = service::list(&self.pool, query).await?;
        Ok(Response::new(pb::ListWorkspaceJobsResponse {
            jobs: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        }))
    }

    async fn tail_job_logs(
        &self,
        request: Request<pb::TailJobLogsRequest>,
    ) -> Result<Response<Self::TailJobLogsStream>, Status> {
        Ok(Response::new(self.tail(request).await?))
    }
}

impl JobsGrpc {
    /// Reads the job id of a request and checks a route permission on its project. An unknown
    /// job gets the answer of a refusal.
    pub(super) async fn job_of(
        &self,
        caller: &Principal,
        id: Option<scylla_proto::common::v1::Id>,
        permission: Permission<Project>,
    ) -> Result<JobId, Error> {
        let id: JobId = parse::required(id, "job_id")?;
        let place: Option<(ProjectId, WorkspaceId)> = service::place(&self.pool, id).await?;
        access::authorize(
            self.gate.as_ref(),
            caller,
            permission,
            format!("job:{id}"),
            place,
        )
        .await?;
        Ok(id)
    }
}
