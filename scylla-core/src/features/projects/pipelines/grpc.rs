//! The gRPC handler of `PipelineService`: authenticate, parse, authorize on the project, call
//! the service, answer. The RPCs of the pipeline files are in `files_grpc.rs`.

use std::sync::Arc;

use scylla_domain::projects::pipelines::{
    ListPipelines, NewPipeline, PipelineAccess, PipelineChange, PipelineHolder, PipelineScope,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Gate, Hooks};
use scylla_proto::pipelines::v1 as pb;
use scylla_proto::pipelines::v1::pipeline_service_server::{
    PipelineService, PipelineServiceServer,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::{convert, service};
use crate::api::parse::{self, ToProto};
use crate::features::identity::agents::Registry;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// Makes the service, with what the Git integration shares.
pub(crate) fn server(ctx: &IntegrationContext) -> PipelineServiceServer<PipelinesGrpc> {
    PipelineServiceServer::new(PipelinesGrpc {
        pool: ctx.pool.clone(),
        hooks: Arc::clone(&ctx.hooks),
        gate: Arc::clone(&ctx.gate),
        registry: Arc::clone(&ctx.registry),
        vcs: ctx.clone(),
    })
}

/// The handler.
pub(crate) struct PipelinesGrpc {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The hooks of the edition.
    hooks: Arc<Hooks>,
    /// The access gate.
    pub(super) gate: Arc<dyn Gate>,
    /// The open streams of the agents.
    registry: Arc<Registry>,
    /// What the Git integration shares: the repository of a run on a commit.
    pub(super) vcs: IntegrationContext,
}

#[tonic::async_trait]
impl PipelineService for PipelinesGrpc {
    async fn create_pipeline(
        &self,
        request: Request<pb::CreatePipelineRequest>,
    ) -> Result<Response<pb::CreatePipelineResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let new = NewPipeline {
            project_id: parse::required(request.project_id, "project_id")?,
            created_by: caller.actor.id,
            name: parse::field(request.name, "name")?,
            nodes: convert::nodes(request.nodes)?,
            layout: request
                .layout
                .map(convert::layout)
                .transpose()?
                .unwrap_or_default(),
            checkout: request.checkout.map(convert::checkout).unwrap_or_default(),
        };

        // Check the route permission on the project, then create the pipeline.
        let workspace = self
            .authorize_in(&caller, PipelineAccess::CREATE_PIPELINE, new.project_id)
            .await?;
        let pipeline =
            service::create(&self.pool, &self.hooks, caller.actor, workspace, new).await?;
        Ok(Response::new(pb::CreatePipelineResponse {
            pipeline: Some(pipeline.to_proto()),
        }))
    }

    async fn get_pipeline(
        &self,
        request: Request<pb::GetPipelineRequest>,
    ) -> Result<Response<pb::GetPipelineResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let id = self
            .pipeline_of(
                &caller,
                request.into_inner().pipeline_id,
                PipelineAccess::GET_PIPELINE,
            )
            .await?;
        let (pipeline, layout) = service::get_with_layout(&self.vcs, id)
            .await?
            .ok_or(Error::NotFound("pipeline"))?;
        Ok(Response::new(pb::GetPipelineResponse {
            pipeline: Some(pipeline.to_proto()),
            layout: layout.map(ToProto::to_proto),
        }))
    }

    async fn save_pipeline_layout(
        &self,
        request: Request<pb::SavePipelineLayoutRequest>,
    ) -> Result<Response<pb::SavePipelineLayoutResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let layout = request
            .layout
            .ok_or_else(|| Error::invalid("layout", "is required"))?;
        let layout = convert::layout(layout)?;

        // Check the route permission, then save the places.
        let permission = PipelineAccess::SAVE_PIPELINE_LAYOUT;
        let id = self
            .pipeline_of(&caller, request.pipeline_id, permission)
            .await?;
        service::save_layout(&self.pool, id, layout).await?;
        Ok(Response::new(pb::SavePipelineLayoutResponse {}))
    }

    async fn update_pipeline(
        &self,
        request: Request<pb::UpdatePipelineRequest>,
    ) -> Result<Response<pb::UpdatePipelineResponse>, Status> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let change = PipelineChange {
            version: request.version,
            name: request
                .name
                .map(|name| parse::field(name, "name"))
                .transpose()?,
            nodes: request
                .nodes
                .map(|list| convert::nodes(list.nodes))
                .transpose()?,
            layout: request
                .layout
                .map(convert::layout)
                .transpose()?
                .unwrap_or_default(),
            checkout: request.checkout.map(convert::checkout),
        };

        // Check the route permission, then change the pipeline.
        let permission = PipelineAccess::UPDATE_PIPELINE;
        let id = self
            .pipeline_of(&caller, request.pipeline_id, permission)
            .await?;
        let pipeline = service::update(&self.pool, id, change).await?;
        Ok(Response::new(pb::UpdatePipelineResponse {
            pipeline: Some(pipeline.to_proto()),
        }))
    }

    async fn delete_pipeline(
        &self,
        request: Request<pb::DeletePipelineRequest>,
    ) -> Result<Response<pb::DeletePipelineResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let permission = PipelineAccess::DELETE_PIPELINE;
        let id = self
            .pipeline_of(&caller, request.into_inner().pipeline_id, permission)
            .await?;
        let actor = caller.actor;
        service::delete(&self.pool, &self.hooks, &self.registry, actor, id).await?;
        Ok(Response::new(pb::DeletePipelineResponse {}))
    }

    async fn list_project_pipelines(
        &self,
        request: Request<pb::ListProjectPipelinesRequest>,
    ) -> Result<Response<pb::ListProjectPipelinesResponse>, Status> {
        // Authenticate the caller, read the request, and check the permission on the project.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let project: ProjectId = parse::required(request.project_id, "project_id")?;
        let page = parse::optional(request.page, "page")?;
        self.authorize_in(&caller, PipelineAccess::LIST_PROJECT_PIPELINES, project)
            .await?;

        // Read one page.
        let query = ListPipelines {
            scope: PipelineScope::Project(project),
            page,
        };
        let page = service::list(&self.vcs, query).await?;
        Ok(Response::new(pb::ListProjectPipelinesResponse {
            pipelines: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        }))
    }

    async fn list_workspace_pipelines(
        &self,
        request: Request<pb::ListWorkspacePipelinesRequest>,
    ) -> Result<Response<pb::ListWorkspacePipelinesResponse>, Status> {
        // Authenticate the caller, read the request, and check the permission on the workspace.
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let page = parse::optional(request.page, "page")?;
        self.gate
            .authorize(
                &caller,
                PipelineHolder::LIST_WORKSPACE_PIPELINES.on(workspace),
            )
            .await
            .map_err(Error::from)?;

        // Read one page.
        let query = ListPipelines {
            scope: PipelineScope::Workspace(workspace),
            page,
        };
        let page = service::list(&self.vcs, query).await?;
        Ok(Response::new(pb::ListWorkspacePipelinesResponse {
            pipelines: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        }))
    }

    async fn run_pipeline(
        &self,
        request: Request<pb::RunPipelineRequest>,
    ) -> Result<Response<pb::RunPipelineResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let request = request.into_inner();
        let git_ref = request.r#ref.map(|value| parse::field(value, "ref"));
        let git_ref = git_ref.transpose()?;
        let permission = PipelineAccess::RUN_PIPELINE;
        let id = self
            .pipeline_of(&caller, request.pipeline_id, permission)
            .await?;
        let job = service::run(&self.vcs, caller.actor, id, git_ref).await?;
        Ok(Response::new(pb::RunPipelineResponse {
            job: Some(job.to_proto()),
        }))
    }

    async fn validate_pipeline_file(
        &self,
        request: Request<pb::ValidatePipelineFileRequest>,
    ) -> Result<Response<pb::ValidatePipelineFileResponse>, Status> {
        self.validate_file(request).await
    }

    async fn export_pipeline(
        &self,
        request: Request<pb::ExportPipelineRequest>,
    ) -> Result<Response<pb::ExportPipelineResponse>, Status> {
        self.export(request).await
    }

    async fn get_file_pipeline(
        &self,
        request: Request<pb::GetFilePipelineRequest>,
    ) -> Result<Response<pb::GetFilePipelineResponse>, Status> {
        self.get_file(request).await
    }

    async fn list_pipeline_files(
        &self,
        request: Request<pb::ListPipelineFilesRequest>,
    ) -> Result<Response<pb::ListPipelineFilesResponse>, Status> {
        self.list_files(request).await
    }

    async fn set_pipeline_source(
        &self,
        request: Request<pb::SetPipelineSourceRequest>,
    ) -> Result<Response<pb::SetPipelineSourceResponse>, Status> {
        self.set_source(request).await
    }
}
