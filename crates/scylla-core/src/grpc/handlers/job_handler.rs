use crate::application::{JobLogUseCases, JobUseCases};
use crate::grpc::adapter::run;
use crate::grpc::mappers::job_to_proto;
use crate::grpc::streaming::spawn_log_forwarder;
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::job::v1::{
    CancelJobRequest, CancelJobResponse, DeleteJobRequest, DeleteJobResponse, GetJobRequest,
    GetJobResponse, ListJobLogsRequest, ListJobLogsResponse, ListJobsRequest, ListJobsResponse,
    ListOrganizationJobsRequest, ListOrganizationJobsResponse, ListPipelineJobsRequest,
    ListPipelineJobsResponse, ListProjectJobsRequest, ListProjectJobsResponse, TailJobLogsRequest,
    TailJobLogsResponse, job_service_server::JobService,
};
use std::pin::Pin;
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct JobHandler {
    actions: Arc<Actions>,
    jobs: Arc<JobUseCases>,
    logs: Arc<JobLogUseCases>,
}

#[async_trait::async_trait]
impl JobService for JobHandler {
    async fn get_job(
        &self,
        request: Request<GetJobRequest>,
    ) -> Result<Response<GetJobResponse>, Status> {
        let job = run(&self.actions, &*self.jobs, request).await?;
        Ok(Response::new(GetJobResponse {
            job: Some(job_to_proto(&job)),
        }))
    }

    async fn cancel_job(
        &self,
        request: Request<CancelJobRequest>,
    ) -> Result<Response<CancelJobResponse>, Status> {
        let job = run(&self.actions, &*self.jobs, request).await?;
        Ok(Response::new(CancelJobResponse {
            job: Some(job_to_proto(&job)),
        }))
    }

    async fn delete_job(
        &self,
        request: Request<DeleteJobRequest>,
    ) -> Result<Response<DeleteJobResponse>, Status> {
        run(&self.actions, &*self.jobs, request).await?;
        Ok(Response::new(DeleteJobResponse {}))
    }

    async fn list_jobs(
        &self,
        request: Request<ListJobsRequest>,
    ) -> Result<Response<ListJobsResponse>, Status> {
        let page = run(&self.actions, &*self.jobs, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_pipeline_jobs(
        &self,
        request: Request<ListPipelineJobsRequest>,
    ) -> Result<Response<ListPipelineJobsResponse>, Status> {
        let page = run(&self.actions, &*self.jobs, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_project_jobs(
        &self,
        request: Request<ListProjectJobsRequest>,
    ) -> Result<Response<ListProjectJobsResponse>, Status> {
        let page = run(&self.actions, &*self.jobs, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_organization_jobs(
        &self,
        request: Request<ListOrganizationJobsRequest>,
    ) -> Result<Response<ListOrganizationJobsResponse>, Status> {
        let page = run(&self.actions, &*self.jobs, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_job_logs(
        &self,
        request: Request<ListJobLogsRequest>,
    ) -> Result<Response<ListJobLogsResponse>, Status> {
        let page = run(&self.actions, &*self.logs, request).await?;
        Ok(Response::new(page.into()))
    }

    type TailJobLogsStream = Pin<
        Box<dyn tokio_stream::Stream<Item = Result<TailJobLogsResponse, Status>> + Send + 'static>,
    >;

    async fn tail_job_logs(
        &self,
        request: Request<TailJobLogsRequest>,
    ) -> Result<Response<Self::TailJobLogsStream>, Status> {
        let stream = run(&self.actions, &*self.logs, request).await?;
        Ok(Response::new(Box::pin(spawn_log_forwarder(stream))))
    }
}
