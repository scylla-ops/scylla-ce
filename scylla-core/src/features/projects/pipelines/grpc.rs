//! The gRPC handler of `PipelineService`: authenticate, parse, authorize on the project, call
//! the service, answer.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::projects::pipelines::{
    ListPipelines, NewPipeline, PipelineAccess, PipelineChange, PipelineHolder, PipelineId,
    PipelineScope,
};
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Gate, Hooks, Principal};
use scylla_proto::pipelines::v1 as pb;
use scylla_proto::pipelines::v1::pipeline_service_server::{
    PipelineService, PipelineServiceServer,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::{access, convert, service};
use crate::api::parse::{self, ToProto};
use crate::features::identity::agents::Registry;
use crate::Error;

/// Makes the service.
pub(crate) fn server(
    pool: PgPool,
    hooks: Arc<Hooks>,
    gate: Arc<dyn Gate>,
    registry: Arc<Registry>,
) -> PipelineServiceServer<PipelinesGrpc> {
    PipelineServiceServer::new(PipelinesGrpc {
        pool,
        hooks,
        gate,
        registry,
    })
}

/// The handler.
pub(crate) struct PipelinesGrpc {
    /// The database pool.
    pool: PgPool,
    /// The hooks of the edition.
    hooks: Arc<Hooks>,
    /// The access gate.
    gate: Arc<dyn Gate>,
    /// The open streams of the agents.
    registry: Arc<Registry>,
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
        let pipeline = service::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound("pipeline"))?;
        Ok(Response::new(pb::GetPipelineResponse {
            pipeline: Some(pipeline.to_proto()),
        }))
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
        let page = service::list(&self.pool, query).await?;
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
        let page = service::list(&self.pool, query).await?;
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
        let permission = PipelineAccess::RUN_PIPELINE;
        let id = self
            .pipeline_of(&caller, request.into_inner().pipeline_id, permission)
            .await?;
        let actor = caller.actor;
        let job = service::run(&self.pool, &self.hooks, &self.registry, actor, id).await?;
        Ok(Response::new(pb::RunPipelineResponse {
            job: Some(job.to_proto()),
        }))
    }
}

impl PipelinesGrpc {
    /// Checks a route permission on a project and on its workspace, and returns the workspace.
    /// An unknown project gets the answer of a refusal.
    async fn authorize_in(
        &self,
        caller: &Principal,
        permission: Permission<Project>,
        id: ProjectId,
    ) -> Result<WorkspaceId, Error> {
        let workspace = service::project_workspace(&self.pool, id).await?;
        let place = workspace.map(|workspace| (id, workspace));
        let object = format!("project:{id}");
        let (_, workspace) =
            access::authorize(self.gate.as_ref(), caller, permission, object, place).await?;
        Ok(workspace)
    }

    /// Reads the pipeline id of a request and checks a route permission on its project. An
    /// unknown pipeline gets the answer of a refusal.
    async fn pipeline_of(
        &self,
        caller: &Principal,
        id: Option<scylla_proto::common::v1::Id>,
        permission: Permission<Project>,
    ) -> Result<PipelineId, Error> {
        let id: PipelineId = parse::required(id, "pipeline_id")?;
        let place = service::place(&self.pool, id).await?;
        let object = format!("pipeline:{id}");
        access::authorize(self.gate.as_ref(), caller, permission, object, place).await?;
        Ok(id)
    }
}
