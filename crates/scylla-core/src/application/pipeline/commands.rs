//! The pipeline's writes. One block per command, in the order it runs: the struct, its
//! access, its payload types, what `Prepare` builds, what `Persist` writes.

use super::PipelineUseCases;
use crate::application::actions::user_or_app;
use crate::application::job::JobScope;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{PipelineId, ProjectId};
use crate::domain::job::{Job, JobOrigin};
use crate::domain::permission::Permission;
use crate::domain::pipeline::{EnvKey, EnvValue, Pipeline, PipelineName, PipelineNode};
use async_trait::async_trait;
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};

#[derive(Debug)]
pub struct CreatePipeline {
    pub project_id: ProjectId,
    pub name: PipelineName,
    pub nodes: Vec<PipelineNode>,
}

impl Describe for CreatePipeline {
    fn access(&self) -> Access {
        Access::Requires(Permission::CreatePipeline(self.project_id.clone()))
    }
}

impl Command for CreatePipeline {
    type Staged = Draft<Pipeline>;
    type Committed = Pipeline;
}

#[async_trait]
impl Run<Prepare<CreatePipeline>> for PipelineUseCases {
    async fn run(
        &self,
        input: Authorized<CreatePipeline>,
    ) -> DomainResult<Prepared<CreatePipeline>> {
        let cmd = input.command();
        self.project_repo.find_by_id(&cmd.project_id).await?;
        let pipeline =
            Pipeline::create(cmd.name.clone(), cmd.project_id.clone(), cmd.nodes.clone())?;
        Ok(input.prepared(Draft::new(pipeline)))
    }
}

#[async_trait]
impl Run<Persist<CreatePipeline>> for PipelineUseCases {
    async fn run(
        &self,
        input: Prepared<CreatePipeline>,
    ) -> DomainResult<Committed<CreatePipeline>> {
        input
            .commit(async |draft| self.pipeline_repo.create(&draft.into_inner()).await)
            .await
    }
}

#[derive(Debug)]
pub struct UpdatePipeline {
    pub id: PipelineId,
    pub name: Option<PipelineName>,
    pub nodes: Option<Vec<PipelineNode>>,
}

impl Describe for UpdatePipeline {
    fn access(&self) -> Access {
        Access::Requires(Permission::UpdatePipeline(self.id.clone()))
    }
}

impl Command for UpdatePipeline {
    type Staged = Draft<Pipeline>;
    type Committed = Pipeline;
}

#[async_trait]
impl Run<Prepare<UpdatePipeline>> for PipelineUseCases {
    async fn run(
        &self,
        input: Authorized<UpdatePipeline>,
    ) -> DomainResult<Prepared<UpdatePipeline>> {
        let cmd = input.command();
        let mut pipeline = self.pipeline_repo.find_by_id(&cmd.id).await?;
        if let Some(name) = &cmd.name {
            pipeline.update_name(name.clone())?;
        }
        if let Some(nodes) = &cmd.nodes {
            pipeline.update_nodes(nodes.clone())?;
        }
        Ok(input.prepared(Draft::new(pipeline)))
    }
}

#[async_trait]
impl Run<Persist<UpdatePipeline>> for PipelineUseCases {
    async fn run(
        &self,
        input: Prepared<UpdatePipeline>,
    ) -> DomainResult<Committed<UpdatePipeline>> {
        input
            .commit(async |draft| self.pipeline_repo.update(&draft.into_inner()).await)
            .await
    }
}

#[derive(Debug)]
pub struct DeletePipeline {
    pub id: PipelineId,
}

impl Describe for DeletePipeline {
    fn access(&self) -> Access {
        Access::Requires(Permission::DeletePipeline(self.id.clone()))
    }
}

impl Command for DeletePipeline {
    type Staged = Pipeline;
    type Committed = Deleted<Pipeline>;
}

#[async_trait]
impl Run<Prepare<DeletePipeline>> for PipelineUseCases {
    async fn run(
        &self,
        input: Authorized<DeletePipeline>,
    ) -> DomainResult<Prepared<DeletePipeline>> {
        let pipeline = self.pipeline_repo.find_by_id(&input.command().id).await?;
        Ok(input.prepared(pipeline))
    }
}

#[async_trait]
impl Run<Persist<DeletePipeline>> for PipelineUseCases {
    async fn run(
        &self,
        input: Prepared<DeletePipeline>,
    ) -> DomainResult<Committed<DeletePipeline>> {
        input
            .commit(async |pipeline| {
                self.dispatch
                    .recall(
                        JobScope::Pipeline(pipeline.id()),
                        self.pipeline_repo.delete(&pipeline),
                    )
                    .await?;
                Ok(Deleted::new(pipeline))
            })
            .await
    }
}

/// The repo reads bypass Cedar, so "run" does not also require "get". The job's origin is the
/// caller. `Prepare` builds the job and checks that it dispatches; the `commit` closure stores
/// it and wakes the dispatcher.
#[derive(Debug)]
pub struct RunPipeline {
    pub id: PipelineId,
}

impl Describe for RunPipeline {
    fn access(&self) -> Access {
        Access::Requires(Permission::RunPipeline(self.id.clone()))
    }
}

impl Command for RunPipeline {
    type Staged = Draft<Job>;
    type Committed = Job;
}

#[async_trait]
impl Run<Prepare<RunPipeline>> for PipelineUseCases {
    async fn run(&self, input: Authorized<RunPipeline>) -> DomainResult<Prepared<RunPipeline>> {
        let origin = user_or_app(input.caller())?;
        let job = self
            .new_job(&input.command().id, origin, Vec::new())
            .await?;
        Ok(input.prepared(Draft::new(job)))
    }
}

#[async_trait]
impl Run<Persist<RunPipeline>> for PipelineUseCases {
    async fn run(&self, input: Prepared<RunPipeline>) -> DomainResult<Committed<RunPipeline>> {
        input
            .commit(async |draft| self.start(&draft.into_inner()).await)
            .await
    }
}

/// A trigger fire, sent as the trigger-runner App of the organization. The origin is the
/// trigger and not the caller, and the inputs come from the trigger; no RPC sends this command.
#[derive(Debug)]
pub struct RunPipelineWithInputs {
    pub id: PipelineId,
    pub inputs: Vec<(EnvKey, EnvValue)>,
    pub origin: JobOrigin,
}

impl Describe for RunPipelineWithInputs {
    fn access(&self) -> Access {
        Access::Requires(Permission::RunPipeline(self.id.clone()))
    }
}

impl Command for RunPipelineWithInputs {
    type Staged = Draft<Job>;
    type Committed = Job;
}

#[async_trait]
impl Run<Prepare<RunPipelineWithInputs>> for PipelineUseCases {
    async fn run(
        &self,
        input: Authorized<RunPipelineWithInputs>,
    ) -> DomainResult<Prepared<RunPipelineWithInputs>> {
        let cmd = input.command();
        let job = self
            .new_job(&cmd.id, cmd.origin.clone(), cmd.inputs.clone())
            .await?;
        Ok(input.prepared(Draft::new(job)))
    }
}

#[async_trait]
impl Run<Persist<RunPipelineWithInputs>> for PipelineUseCases {
    async fn run(
        &self,
        input: Prepared<RunPipelineWithInputs>,
    ) -> DomainResult<Committed<RunPipelineWithInputs>> {
        input
            .commit(async |draft| self.start(&draft.into_inner()).await)
            .await
    }
}
