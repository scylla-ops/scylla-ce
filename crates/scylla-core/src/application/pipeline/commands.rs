//! The pipeline's writes. Each command, its access and its payload types. The strata module of the
//! slice runs it.

use crate::domain::ids::{PipelineId, ProjectId};
use crate::domain::job::{Job, JobOrigin};
use crate::domain::permission::Permission;
use crate::domain::pipeline::{EnvKey, EnvValue, Pipeline, PipelineName, PipelineNode};
use scylla_extension::{Access, Command, Deleted, Describe, Draft};

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

/// The repo reads bypass Cedar, so "run" does not also require "get". The job's origin is the
/// caller. The use case builds the job and checks that it dispatches, then stores it and wakes
/// the dispatcher.
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
