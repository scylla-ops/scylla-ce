use super::{authed, parse};
use crate::{Pipelines, PipelinesRpc};
use scylla_core::grpc::convert::wrap;
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::pipeline_to_proto;
use scylla_proto::pipeline::v1 as pb;
use strata_tonic::Grpc;

pub use super::service::pipeline::pipeline_service_client::PipelineServiceClient;
pub use super::service::pipeline::pipeline_service_server::{
    PipelineService, PipelineServiceServer,
};

strata_tonic::service! {
    impl<A: Pipelines> PipelineService for Grpc<A> {
        rpc create_pipeline(pb::CreatePipelineRequest) -> pb::CreatePipelineResponse = |call, req| {
            authed(&call).await?;
            let (project_id, name, nodes) = parse(req)?;
            let pipeline = PipelinesRpc::create_pipeline(&call.scoped(), project_id, name, nodes).await.map_err(domain_error_to_status)?;
            Ok(pb::CreatePipelineResponse { pipeline: Some(pipeline_to_proto(&pipeline)) })
        }
        rpc get_pipeline(pb::GetPipelineRequest) -> pb::GetPipelineResponse = |call, req| {
            authed(&call).await?;
            let id = parse(req)?;
            let pipeline = PipelinesRpc::get_pipeline(&call.scoped(), id).await.map_err(domain_error_to_status)?;
            Ok(pb::GetPipelineResponse { pipeline: Some(pipeline_to_proto(&pipeline)) })
        }
        rpc update_pipeline(pb::UpdatePipelineRequest) -> pb::UpdatePipelineResponse = |call, req| {
            authed(&call).await?;
            let (id, name, nodes) = parse(req)?;
            let pipeline = PipelinesRpc::update_pipeline(&call.scoped(), id, name, nodes).await.map_err(domain_error_to_status)?;
            Ok(pb::UpdatePipelineResponse { pipeline: Some(pipeline_to_proto(&pipeline)) })
        }
        rpc delete_pipeline(pb::DeletePipelineRequest) -> pb::DeletePipelineResponse = |call, req| {
            authed(&call).await?;
            let id = parse(req)?;
            PipelinesRpc::delete_pipeline(&call.scoped(), id).await.map_err(domain_error_to_status)?;
            Ok(pb::DeletePipelineResponse {})
        }
        rpc list_pipelines(pb::ListPipelinesRequest) -> pb::ListPipelinesResponse = |call, req| {
            authed(&call).await?;
            let pagination = parse(req)?;
            Ok(PipelinesRpc::list_pipelines(&call.scoped(), pagination).await.map_err(domain_error_to_status)?.into())
        }
        rpc list_project_pipelines(pb::ListProjectPipelinesRequest) -> pb::ListProjectPipelinesResponse = |call, req| {
            authed(&call).await?;
            let (project_id, pagination) = parse(req)?;
            Ok(PipelinesRpc::list_project_pipelines(&call.scoped(), project_id, pagination).await.map_err(domain_error_to_status)?.into())
        }
        rpc list_organization_pipelines(pb::ListOrganizationPipelinesRequest) -> pb::ListOrganizationPipelinesResponse = |call, req| {
            authed(&call).await?;
            let (organization_id, pagination) = parse(req)?;
            Ok(PipelinesRpc::list_organization_pipelines(&call.scoped(), organization_id, pagination).await.map_err(domain_error_to_status)?.into())
        }
        rpc run_pipeline(pb::RunPipelineRequest) -> pb::RunPipelineResponse = |call, req| {
            authed(&call).await?;
            let id = parse(req)?;
            let job = PipelinesRpc::run_pipeline(&call.scoped(), id).await.map_err(domain_error_to_status)?;
            Ok(pb::RunPipelineResponse { job_id: wrap(job.id().to_string()) })
        }
    }
}
