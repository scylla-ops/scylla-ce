use super::{authed, parse};
use crate::{AccessRpc, JobControl, JobControlRpc};
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::job_to_proto;
use scylla_core::grpc::streaming::spawn_log_forwarder;
use scylla_proto::job::v1 as pb;
use std::pin::Pin;
use strata_tonic::{Grpc, Rpc};
use tonic::codegen::tokio_stream::Stream;
use tonic::{Request, Response, Status};

pub use super::service::job::job_service_client::JobServiceClient;
pub use super::service::job::job_service_server::{JobService, JobServiceServer};

fn rpc(method: &'static str) -> Rpc {
    Rpc::new("JobService", method)
}

#[tonic::async_trait]
impl<A: JobControl> JobService for Grpc<A> {
    async fn get_job(
        &self,
        request: Request<pb::GetJobRequest>,
    ) -> Result<Response<pb::GetJobResponse>, Status> {
        self.unary_rpc(rpc("get_job"), request, |call, req| async move {
            authed(&call).await?;
            let job = JobControlRpc::get_job(&call.scoped(), parse(req)?)
                .await
                .map_err(domain_error_to_status)?;
            Ok(pb::GetJobResponse {
                job: Some(job_to_proto(&job)),
            })
        })
        .await
    }

    async fn cancel_job(
        &self,
        request: Request<pb::CancelJobRequest>,
    ) -> Result<Response<pb::CancelJobResponse>, Status> {
        self.unary_rpc(rpc("cancel_job"), request, |call, req| async move {
            authed(&call).await?;
            let job = JobControlRpc::cancel_job(&call.scoped(), parse(req)?)
                .await
                .map_err(domain_error_to_status)?;
            Ok(pb::CancelJobResponse {
                job: Some(job_to_proto(&job)),
            })
        })
        .await
    }

    async fn delete_job(
        &self,
        request: Request<pb::DeleteJobRequest>,
    ) -> Result<Response<pb::DeleteJobResponse>, Status> {
        self.unary_rpc(rpc("delete_job"), request, |call, req| async move {
            authed(&call).await?;
            JobControlRpc::delete_job(&call.scoped(), parse(req)?)
                .await
                .map_err(domain_error_to_status)?;
            Ok(pb::DeleteJobResponse {})
        })
        .await
    }

    async fn list_jobs(
        &self,
        request: Request<pb::ListJobsRequest>,
    ) -> Result<Response<pb::ListJobsResponse>, Status> {
        self.unary_rpc(rpc("list_jobs"), request, |call, req| async move {
            authed(&call).await?;
            Ok(JobControlRpc::list_jobs(&call.scoped(), parse(req)?)
                .await
                .map_err(domain_error_to_status)?
                .into())
        })
        .await
    }

    async fn list_pipeline_jobs(
        &self,
        request: Request<pb::ListPipelineJobsRequest>,
    ) -> Result<Response<pb::ListPipelineJobsResponse>, Status> {
        self.unary_rpc(rpc("list_pipeline_jobs"), request, |call, req| async move {
            authed(&call).await?;
            Ok(
                JobControlRpc::list_pipeline_jobs(&call.scoped(), parse(req)?)
                    .await
                    .map_err(domain_error_to_status)?
                    .into(),
            )
        })
        .await
    }

    async fn list_project_jobs(
        &self,
        request: Request<pb::ListProjectJobsRequest>,
    ) -> Result<Response<pb::ListProjectJobsResponse>, Status> {
        self.unary_rpc(rpc("list_project_jobs"), request, |call, req| async move {
            authed(&call).await?;
            Ok(
                JobControlRpc::list_project_jobs(&call.scoped(), parse(req)?)
                    .await
                    .map_err(domain_error_to_status)?
                    .into(),
            )
        })
        .await
    }

    async fn list_organization_jobs(
        &self,
        request: Request<pb::ListOrganizationJobsRequest>,
    ) -> Result<Response<pb::ListOrganizationJobsResponse>, Status> {
        self.unary_rpc(
            rpc("list_organization_jobs"),
            request,
            |call, req| async move {
                authed(&call).await?;
                Ok(
                    JobControlRpc::list_organization_jobs(&call.scoped(), parse(req)?)
                        .await
                        .map_err(domain_error_to_status)?
                        .into(),
                )
            },
        )
        .await
    }

    async fn list_job_logs(
        &self,
        request: Request<pb::ListJobLogsRequest>,
    ) -> Result<Response<pb::ListJobLogsResponse>, Status> {
        self.unary_rpc(rpc("list_job_logs"), request, |call, req| async move {
            authed(&call).await?;
            Ok(JobControlRpc::list_job_logs(&call.scoped(), parse(req)?)
                .await
                .map_err(domain_error_to_status)?
                .into())
        })
        .await
    }

    type TailJobLogsStream =
        Pin<Box<dyn Stream<Item = Result<pb::TailJobLogsResponse, Status>> + Send + 'static>>;

    async fn tail_job_logs(
        &self,
        request: Request<pb::TailJobLogsRequest>,
    ) -> Result<Response<Self::TailJobLogsStream>, Status> {
        let ctx = self.stream_context(&request).await?;
        let scoped = self.server().scoped(&ctx);
        AccessRpc::authenticated(&scoped)
            .await
            .map_err(domain_error_to_status)?;
        let tail = JobControlRpc::tail_job_logs(&scoped, parse(request.into_inner())?)
            .await
            .map_err(domain_error_to_status)?;
        Ok(Response::new(Box::pin(spawn_log_forwarder(tail))))
    }
}
