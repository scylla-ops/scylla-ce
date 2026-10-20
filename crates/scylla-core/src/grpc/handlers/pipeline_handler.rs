//! The adapter: each RPC is one `run` and its response. Parsing lives in the
//! pipeline mapper, behind `Parse`; no RPC checks a permission or touches a port.

use crate::application::PipelineUseCases;
use crate::grpc::adapter::run;
use crate::grpc::convert::wrap;
use crate::grpc::mappers::pipeline_to_proto;
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::pipeline::v1::{
    CreatePipelineRequest, CreatePipelineResponse, DeletePipelineRequest, DeletePipelineResponse,
    GetPipelineRequest, GetPipelineResponse, ListOrganizationPipelinesRequest,
    ListOrganizationPipelinesResponse, ListPipelinesRequest, ListPipelinesResponse,
    ListProjectPipelinesRequest, ListProjectPipelinesResponse, RunPipelineRequest,
    RunPipelineResponse, UpdatePipelineRequest, UpdatePipelineResponse,
    pipeline_service_server::PipelineService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct PipelineHandler {
    actions: Arc<Actions>,
    pipelines: Arc<PipelineUseCases>,
}

#[async_trait::async_trait]
impl PipelineService for PipelineHandler {
    async fn create_pipeline(
        &self,
        request: Request<CreatePipelineRequest>,
    ) -> Result<Response<CreatePipelineResponse>, Status> {
        let pipeline = run(&self.actions, &*self.pipelines, request).await?;
        Ok(Response::new(CreatePipelineResponse {
            pipeline: Some(pipeline_to_proto(&pipeline)),
        }))
    }

    async fn get_pipeline(
        &self,
        request: Request<GetPipelineRequest>,
    ) -> Result<Response<GetPipelineResponse>, Status> {
        let pipeline = run(&self.actions, &*self.pipelines, request).await?;
        Ok(Response::new(GetPipelineResponse {
            pipeline: Some(pipeline_to_proto(&pipeline)),
        }))
    }

    async fn update_pipeline(
        &self,
        request: Request<UpdatePipelineRequest>,
    ) -> Result<Response<UpdatePipelineResponse>, Status> {
        let pipeline = run(&self.actions, &*self.pipelines, request).await?;
        Ok(Response::new(UpdatePipelineResponse {
            pipeline: Some(pipeline_to_proto(&pipeline)),
        }))
    }

    async fn delete_pipeline(
        &self,
        request: Request<DeletePipelineRequest>,
    ) -> Result<Response<DeletePipelineResponse>, Status> {
        run(&self.actions, &*self.pipelines, request).await?;
        Ok(Response::new(DeletePipelineResponse {}))
    }

    async fn list_pipelines(
        &self,
        request: Request<ListPipelinesRequest>,
    ) -> Result<Response<ListPipelinesResponse>, Status> {
        let page = run(&self.actions, &*self.pipelines, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_project_pipelines(
        &self,
        request: Request<ListProjectPipelinesRequest>,
    ) -> Result<Response<ListProjectPipelinesResponse>, Status> {
        let page = run(&self.actions, &*self.pipelines, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_organization_pipelines(
        &self,
        request: Request<ListOrganizationPipelinesRequest>,
    ) -> Result<Response<ListOrganizationPipelinesResponse>, Status> {
        let page = run(&self.actions, &*self.pipelines, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn run_pipeline(
        &self,
        request: Request<RunPipelineRequest>,
    ) -> Result<Response<RunPipelineResponse>, Status> {
        let job = run(&self.actions, &*self.pipelines, request).await?;
        Ok(Response::new(RunPipelineResponse {
            job_id: wrap(job.id().to_string()),
        }))
    }
}
