//! The gRPC handler of `VcsReportService`: authenticate, parse, authorize on the project of each
//! job, call the service, answer.

use std::collections::HashMap;

use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::pipelines::PipelineAccess;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Principal;
use scylla_proto::integrations::v1 as pb;
use scylla_proto::integrations::v1::vcs_report_service_server::{
    VcsReportService, VcsReportServiceServer,
};
use tonic::{Request, Response, Status};

use super::{convert, service};
use crate::api::parse;
use crate::features::integrations::IntegrationContext;
use crate::features::projects::pipelines::access;
use crate::Error;

/// The most jobs of one list.
const MAX_JOBS: usize = 100;

/// Makes the service.
pub(super) fn server(ctx: IntegrationContext) -> VcsReportServiceServer<ReportsGrpc> {
    VcsReportServiceServer::new(ReportsGrpc { ctx })
}

/// The handler.
pub(super) struct ReportsGrpc {
    /// What the parts of the integration share.
    ctx: IntegrationContext,
}

#[tonic::async_trait]
impl VcsReportService for ReportsGrpc {
    async fn list_job_reports(
        &self,
        request: Request<pb::ListJobReportsRequest>,
    ) -> Result<Response<pb::ListJobReportsResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn retry_job_report(
        &self,
        request: Request<pb::RetryJobReportRequest>,
    ) -> Result<Response<pb::RetryJobReportResponse>, Status> {
        Ok(Response::new(self.retry(request).await?))
    }
}

impl ReportsGrpc {
    /// Handles `ListJobReports`: the reports of the jobs that the caller can read, in the order of
    /// the request. With no job that the caller can read, the answer is a refusal.
    async fn list(
        &self,
        request: Request<pb::ListJobReportsRequest>,
    ) -> Result<pb::ListJobReportsResponse, Error> {
        // Authenticate the caller, then read 1 to 100 jobs.
        let caller = self.ctx.gate.caller(&request).await?;
        let ids = request.into_inner().job_ids;
        if ids.is_empty() || ids.len() > MAX_JOBS {
            return Err(Error::invalid("job_ids", "must have 1 to 100 jobs"));
        }
        let mut jobs: Vec<JobId> = Vec::with_capacity(ids.len());
        for id in ids {
            let job = parse::field(id, "job_ids")?;
            if !jobs.contains(&job) {
                jobs.push(job);
            }
        }

        // Check the route permission one time on the project of each known job.
        let places = service::places(&self.ctx, &jobs).await?;
        let mut checked: HashMap<ProjectId, bool> = HashMap::new();
        let mut refusal = None;
        let mut visible = Vec::new();
        for job in &jobs {
            let place = places.iter().find(|(id, ..)| id == job);
            let place = place.map(|(_, project, workspace)| (*project, *workspace));
            if let Some(passed) = place.and_then(|(project, _)| checked.get(&project).copied()) {
                if passed {
                    visible.push(*job);
                }
                continue;
            }
            match self.authorize(&caller, *job, place).await {
                Ok(project) => {
                    checked.insert(project, true);
                    visible.push(*job);
                }
                Err(error) => {
                    if let Some((project, _)) = place {
                        checked.insert(project, false);
                    }
                    refusal.get_or_insert(error);
                }
            }
        }

        // Refuse a list with no job that the caller can read, else answer.
        if visible.is_empty() {
            return Err(refusal.unwrap_or_else(|| Error::NotFound("job")));
        }
        let listed = service::list(&self.ctx, &visible).await?;
        Ok(pb::ListJobReportsResponse {
            jobs: listed.into_iter().map(convert::job_reports).collect(),
        })
    }

    /// Handles `RetryJobReport`: a report that retries or that stopped is due at once.
    async fn retry(
        &self,
        request: Request<pb::RetryJobReportRequest>,
    ) -> Result<pb::RetryJobReportResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let job: JobId = parse::required(request.job_id, "job_id")?;
        if request.context.trim().is_empty() {
            return Err(Error::invalid("context", "is required"));
        }

        // Check the route permission on the project of the job, then send the report again.
        let places = service::places(&self.ctx, &[job]).await?;
        let place = places
            .first()
            .map(|(_, project, workspace)| (*project, *workspace));
        let permission = PipelineAccess::RETRY_JOB_REPORT;
        let object = format!("job {job}");
        let gate = self.ctx.gate.as_ref();
        access::authorize(gate, &caller, permission, object, place).await?;
        let listed = service::retry(&self.ctx, job, &request.context).await?;
        Ok(pb::RetryJobReportResponse {
            report: Some(convert::report(listed)),
        })
    }

    /// Checks `list_job_reports` on the project of a job, and returns the project.
    async fn authorize(
        &self,
        caller: &Principal,
        job: JobId,
        place: Option<(ProjectId, WorkspaceId)>,
    ) -> Result<ProjectId, Error> {
        let permission = PipelineAccess::LIST_JOB_REPORTS;
        let gate = self.ctx.gate.as_ref();
        let object = format!("job {job}");
        let (project, _) = access::authorize(gate, caller, permission, object, place).await?;
        Ok(project)
    }
}
