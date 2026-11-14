//! The start of a job: a run of a pipeline, a fire of a trigger, or a new run of a job.

use super::{Job, JobId, NewJob, PipelineFileRef};
use crate::identity::actors::ActorId;
use crate::integrations::sources::{JobSource, SourceEvent};
use crate::projects::pipeline_files::{CheckoutSettings, FileError};
use crate::projects::pipelines::{EnvValue, EnvVar, Node, Pipeline, PipelineId};
use crate::projects::triggers::TriggerId;
use crate::projects::ProjectId;
use crate::tenancy::workspaces::WorkspaceId;

/// The error of a job of an invalid pipeline file.
pub const INVALID_FILE: &str = "the pipeline file is not valid";

/// Who asked for a new run of a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RerunOrigin {
    /// A person, in Scylla.
    Person,
    /// A person on the provider, for example with Re-run on GitHub.
    Provider,
}

/// The input of a job start: a run of a pipeline, a fire of a trigger, or a new run of a job.
#[derive(Clone, Debug)]
pub struct StartJob {
    /// The pipeline.
    pub pipeline_id: PipelineId,
    /// The project of the pipeline.
    pub project_id: ProjectId,
    /// The workspace of the project.
    pub workspace_id: WorkspaceId,
    /// The nodes to run.
    pub nodes: Vec<Node>,
    /// The variables that each node gets, if it has no variable with the same name.
    pub variables: Vec<EnvVar>,
    /// The trigger that starts the job.
    pub trigger_id: Option<TriggerId>,
    /// The job that this job runs again.
    pub rerun_of: Option<JobId>,
    /// The commit and the event of the job.
    pub source: Option<JobSource>,
    /// The pipeline file of a job of a file pipeline.
    pub pipeline_file: Option<PipelineFileRef>,
    /// The errors of an invalid pipeline file.
    pub file_errors: Vec<FileError>,
    /// The checkout settings of the pipeline. Only a job with a source has a checkout.
    pub checkout: CheckoutSettings,
}

impl StartJob {
    /// Makes a run of the nodes of a pipeline with its checkout settings, with no variable, no
    /// trigger and no source.
    pub fn of(pipeline: Pipeline) -> Self {
        Self {
            pipeline_id: pipeline.id,
            project_id: pipeline.project_id,
            workspace_id: pipeline.workspace_id,
            nodes: pipeline.nodes,
            variables: Vec::new(),
            trigger_id: None,
            rerun_of: None,
            source: None,
            pipeline_file: None,
            file_errors: Vec::new(),
            checkout: pipeline.checkout,
        }
    }

    /// Makes a new run of a job: its pipeline, its nodes, its trigger, its pipeline file, its
    /// source and its checkout. The new source is not automatic, and a rerun from the provider
    /// has its event.
    pub fn rerun(job: Job, origin: RerunOrigin) -> Self {
        let source = job.source.map(|mut source| {
            source.automatic = false;
            if origin == RerunOrigin::Provider {
                source.event = SourceEvent::ProviderRerun;
            }
            source
        });
        Self {
            pipeline_id: job.pipeline_id,
            project_id: job.project_id,
            workspace_id: job.workspace_id,
            nodes: job.nodes,
            variables: Vec::new(),
            trigger_id: job.trigger_id,
            rerun_of: Some(job.id),
            source,
            pipeline_file: job.pipeline_file,
            file_errors: job.file_errors,
            checkout: job.checkout.map_or_else(no_checkout, |run| run.settings),
        }
    }

    /// Returns the names of the secrets that the nodes name.
    pub fn secrets(&self) -> Vec<String> {
        self.nodes
            .iter()
            .flat_map(|node| &node.env)
            .filter_map(|var| match &var.value {
                EnvValue::Secret(name) => Some(name.as_str().to_owned()),
                EnvValue::Literal(_) => None,
            })
            .collect()
    }

    /// Makes the job that an actor creates: each node gets the variables that it does not have,
    /// a job from a fork waits for an approval, and a job with a source gets the code when its
    /// checkout is on.
    pub fn into_new(self, created_by: ActorId) -> NewJob {
        let mut nodes = self.nodes;
        for node in &mut nodes {
            for var in &self.variables {
                if !node.env.iter().any(|own| own.name == var.name) {
                    node.env.push(var.clone());
                }
            }
        }
        let awaiting = self.source.as_ref().is_some_and(JobSource::from_fork);
        let checkout = (self.source.is_some() && self.checkout.enabled).then_some(self.checkout);
        NewJob {
            pipeline_id: self.pipeline_id,
            project_id: self.project_id,
            workspace_id: self.workspace_id,
            created_by,
            rerun_of: self.rerun_of,
            trigger_id: self.trigger_id,
            nodes,
            source: self.source,
            awaiting,
            pipeline_file: self.pipeline_file,
            file_errors: self.file_errors,
            checkout,
        }
    }
}

/// Returns the settings of a job with no checkout.
fn no_checkout() -> CheckoutSettings {
    CheckoutSettings {
        enabled: false,
        ..CheckoutSettings::default()
    }
}
