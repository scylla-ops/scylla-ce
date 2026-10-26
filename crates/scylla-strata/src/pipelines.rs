use crate::access::CallerOf;
use crate::ports::JobDispatch;
use scylla_core::application::actions::user_or_app;
use scylla_core::application::agent::dispatch::assemble_dispatch;
use scylla_core::application::job::JobScope;
use scylla_core::application::pagination::PaginatedResult;
use scylla_core::application::pipeline::{
    CreatePipeline, DeletePipeline, GetPipeline, ListOrganizationPipelines, ListPipelines,
    ListProjectPipelines, RunPipeline, RunPipelineWithInputs, UpdatePipeline,
};
use scylla_core::application::{
    DispatchUseCases, JobRepository, PipelineRepository, ProjectRepository, SecretResolver,
};
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::PipelineId;
use scylla_domain::domain::job::{Job, JobOrigin};
use scylla_domain::domain::pipeline::{EnvKey, EnvValue, Pipeline};
use scylla_extension::Describe;
use std::sync::Arc;
use strata::{BuildCx, BuildError, Component};

pub struct Dispatcher(Arc<DispatchUseCases>);

impl Component for Dispatcher {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(Dispatcher(cx.resource::<JobDispatch>().await?.0))
    }
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

    #[usecase(write, access = cmd.access())]
    async fn create_pipeline(&self, cmd: CreatePipeline) -> DomainResult<Pipeline> {
        self.pipeline_projects().find_by_id(&cmd.project_id).await?;
        let pipeline = Pipeline::create(cmd.name, cmd.project_id, cmd.nodes)?;
        self.pipeline_repo().create(&pipeline).await
    }

    #[usecase(write, access = cmd.access())]
    async fn update_pipeline(&self, cmd: UpdatePipeline) -> DomainResult<Pipeline> {
        let mut pipeline = self.pipeline_repo().find_by_id(&cmd.id).await?;
        if let Some(name) = cmd.name {
            pipeline.update_name(name)?;
        }
        if let Some(nodes) = cmd.nodes {
            pipeline.update_nodes(nodes)?;
        }
        self.pipeline_repo().update(&pipeline).await
    }

    #[usecase(write, access = cmd.access())]
    async fn delete_pipeline(&self, cmd: DeletePipeline) -> DomainResult<()> {
        let pipeline = self.pipeline_repo().find_by_id(&cmd.id).await?;
        self.pipeline_dispatch()
            .0
            .recall(
                JobScope::Pipeline(pipeline.id()),
                self.pipeline_repo().delete(&pipeline),
            )
            .await
    }

    #[usecase(write, access = cmd.access())]
    async fn run_pipeline(&self, cmd: RunPipeline) -> DomainResult<Job> {
        let origin = user_or_app(self.ctx().caller())?;
        let job = self.new_job(&cmd.id, origin, Vec::new()).await?;
        self.start(&job).await
    }

    #[usecase(write, access = cmd.access())]
    async fn run_pipeline_with_inputs(&self, cmd: RunPipelineWithInputs) -> DomainResult<Job> {
        let job = self.new_job(&cmd.id, cmd.origin, cmd.inputs).await?;
        self.start(&job).await
    }

    #[usecase(read, access = query.access())]
    async fn get_pipeline(&self, query: GetPipeline) -> DomainResult<Pipeline> {
        self.pipeline_repo().find_by_id(&query.id).await
    }

    #[usecase(read, access = query.access())]
    async fn list_pipelines(
        &self,
        query: ListPipelines,
    ) -> DomainResult<PaginatedResult<Pipeline>> {
        self.pipeline_repo()
            .list_all(query.pagination.as_ref())
            .await
    }

    #[usecase(read, access = query.access())]
    async fn list_project_pipelines(
        &self,
        query: ListProjectPipelines,
    ) -> DomainResult<PaginatedResult<Pipeline>> {
        self.pipeline_repo()
            .list_by_project(&query.project_id, query.pagination.as_ref())
            .await
    }

    #[usecase(read, access = query.access())]
    async fn list_organization_pipelines(
        &self,
        query: ListOrganizationPipelines,
    ) -> DomainResult<PaginatedResult<Pipeline>> {
        self.pipeline_repo()
            .list_by_organization(&query.organization_id, query.pagination.as_ref())
            .await
    }
}
