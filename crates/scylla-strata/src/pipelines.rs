use crate::access::CallerOf;
use crate::ports::JobDispatch;
use scylla_core::application::agent::dispatch::assemble_dispatch;
use scylla_core::application::guards::user_or_app;
use scylla_core::application::job::JobScope;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::{
    DispatchUseCases, JobRepository, PipelineRepository, ProjectRepository, SecretResolver,
};
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::{OrganizationId, PipelineId, ProjectId};
use scylla_domain::domain::job::{Job, JobOrigin};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::pipeline::{EnvKey, EnvValue, Pipeline, PipelineName, PipelineNode};
use scylla_extension::Access;
use std::sync::Arc;
use strata::{BuildCx, BuildError, Component};

pub struct Dispatcher(Arc<DispatchUseCases>);

impl Component for Dispatcher {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(Dispatcher(cx.resource::<JobDispatch>().await?.0))
    }
}

/// The access of a run of the pipeline `id`: by a caller, or by the runner of a trigger fire.
pub(crate) fn run_access(id: &PipelineId) -> Access {
    Access::Requires(Permission::RunPipeline(id.clone()))
}

#[strata::module(section = "pipelines")]
pub trait Pipelines: crate::Access {
    #[component]
    type PipelineRepo = Arc<dyn PipelineRepository>;

    #[component]
    type PipelineProjects = Arc<dyn ProjectRepository>;

    #[component]
    type PipelineJobs = Arc<dyn JobRepository>;

    #[component]
    type PipelineSecrets = Arc<dyn SecretResolver>;

    #[component]
    type PipelineDispatch = Dispatcher;

    async fn new_job(
        &self,
        id: &PipelineId,
        origin: JobOrigin,
        inputs: Vec<(EnvKey, EnvValue)>,
    ) -> DomainResult<Job> {
        let pipeline = self.pipeline_repo().find_by_id(id).await?;
        let job = Job::create_from_pipeline(&pipeline, origin).with_inputs(inputs);
        assemble_dispatch(&**self.pipeline_secrets(), pipeline.project_id(), &job).await?;
        Ok(job)
    }

    async fn start(&self, job: &Job) -> DomainResult<Job> {
        let job = self.pipeline_jobs().create(job).await?;
        self.pipeline_dispatch().0.wake(None);
        Ok(job)
    }

    #[usecase(write, access = Access::Requires(Permission::CreatePipeline(project_id.clone())))]
    async fn create_pipeline(
        &self,
        project_id: ProjectId,
        name: PipelineName,
        nodes: Vec<PipelineNode>,
    ) -> DomainResult<Pipeline> {
        self.pipeline_projects().find_by_id(&project_id).await?;
        let pipeline = Pipeline::create(name, project_id, nodes)?;
        self.pipeline_repo().create(&pipeline).await
    }

    #[usecase(write, access = Access::Requires(Permission::UpdatePipeline(id.clone())))]
    async fn update_pipeline(
        &self,
        id: PipelineId,
        name: Option<PipelineName>,
        nodes: Option<Vec<PipelineNode>>,
    ) -> DomainResult<Pipeline> {
        let mut pipeline = self.pipeline_repo().find_by_id(&id).await?;
        if let Some(name) = name {
            pipeline.update_name(name)?;
        }
        if let Some(nodes) = nodes {
            pipeline.update_nodes(nodes)?;
        }
        self.pipeline_repo().update(&pipeline).await
    }

    #[usecase(write, access = Access::Requires(Permission::DeletePipeline(id.clone())))]
    async fn delete_pipeline(&self, id: PipelineId) -> DomainResult<()> {
        let pipeline = self.pipeline_repo().find_by_id(&id).await?;
        self.pipeline_dispatch()
            .0
            .recall(
                JobScope::Pipeline(pipeline.id()),
                self.pipeline_repo().delete(&pipeline),
            )
            .await
    }

    /// The repo reads bypass Cedar, thus "run" does not also require "get". The origin of the
    /// job is the caller. A trigger fire does not come here: it runs `new_job` and `start` as
    /// the runner of the trigger, with the same access (`run_access`).
    #[usecase(write, access = run_access(&id))]
    async fn run_pipeline(&self, id: PipelineId) -> DomainResult<Job> {
        let origin = user_or_app(self.ctx().caller())?;
        let job = self.new_job(&id, origin, Vec::new()).await?;
        self.start(&job).await
    }

    #[usecase(read, access = Access::Requires(Permission::ReadPipeline(id.clone())))]
    async fn get_pipeline(&self, id: PipelineId) -> DomainResult<Pipeline> {
        self.pipeline_repo().find_by_id(&id).await
    }

    #[usecase(read, access = Access::Requires(Permission::ListPipelines))]
    async fn list_pipelines(
        &self,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Pipeline>> {
        self.pipeline_repo().list_all(pagination.as_ref()).await
    }

    #[usecase(read, access = Access::Requires(Permission::ListPipelinesByProject(project_id.clone())))]
    async fn list_project_pipelines(
        &self,
        project_id: ProjectId,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Pipeline>> {
        self.pipeline_repo()
            .list_by_project(&project_id, pagination.as_ref())
            .await
    }

    #[usecase(read, access = Access::Requires(Permission::ListPipelinesByOrganization(organization_id.clone())))]
    async fn list_organization_pipelines(
        &self,
        organization_id: OrganizationId,
        pagination: Option<PaginationParams>,
    ) -> DomainResult<PaginatedResult<Pipeline>> {
        self.pipeline_repo()
            .list_by_organization(&organization_id, pagination.as_ref())
            .await
    }
}
