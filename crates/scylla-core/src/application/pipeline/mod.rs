pub mod commands;
pub mod queries;
pub mod repository;

pub use commands::{
    CreatePipeline, DeletePipeline, RunPipeline, RunPipelineWithInputs, UpdatePipeline,
};
pub use queries::{GetPipeline, ListOrganizationPipelines, ListPipelines, ListProjectPipelines};
pub use repository::PipelineRepository;

use crate::application::agent::dispatch::assemble_dispatch;
use crate::application::{DispatchUseCases, JobRepository, ProjectRepository, SecretResolver};
use crate::domain::errors::DomainResult;
use crate::domain::ids::PipelineId;
use crate::domain::job::{Job, JobOrigin};
use crate::domain::pipeline::{EnvKey, EnvValue};
use derive_more::Constructor;
use std::sync::Arc;

/// The pipeline aggregate's stage runners, one block per action in `commands.rs` and
/// `queries.rs`. A run stores its job and wakes the dispatcher through `dispatch`, which also
/// stops the live jobs of a deleted pipeline.
#[derive(Constructor)]
pub struct PipelineUseCases {
    pub(super) pipeline_repo: Arc<dyn PipelineRepository>,
    pub(super) project_repo: Arc<dyn ProjectRepository>,
    pub(super) job_repo: Arc<dyn JobRepository>,
    pub(super) secret_resolver: Arc<dyn SecretResolver>,
    pub(super) dispatch: Arc<DispatchUseCases>,
}

impl PipelineUseCases {
    /// A job that dispatches as it is: each secret resolves and the resolved job fits the
    /// stream, else the run fails before it stores anything.
    async fn new_job(
        &self,
        id: &PipelineId,
        origin: JobOrigin,
        inputs: Vec<(EnvKey, EnvValue)>,
    ) -> DomainResult<Job> {
        let pipeline = self.pipeline_repo.find_by_id(id).await?;
        let job = Job::create_from_pipeline(&pipeline, origin).with_inputs(inputs);
        assemble_dispatch(&*self.secret_resolver, pipeline.project_id(), &job).await?;
        Ok(job)
    }

    async fn start(&self, job: &Job) -> DomainResult<Job> {
        let job = self.job_repo.create(job).await?;
        self.dispatch.wake(None);
        Ok(job)
    }
}

#[cfg(test)]
mod tests;
