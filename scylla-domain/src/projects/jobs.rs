//! The jobs: one run of a pipeline on an agent, with a snapshot of its nodes and the state of
//! each node.

mod logs;
mod state;
#[cfg(test)]
mod tests;

pub use logs::{LogBatch, LogBatchId, LogLine, LogStream};
pub use state::{JobEvent, NodeRun};
use time::OffsetDateTime;

use crate::identity::actors::ActorId;
use crate::identity::apps::AppId;
use crate::projects::pipelines::{Node, PipelineId};
use crate::projects::ProjectId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Id, Page, PageRequest, ResourceKind};

/// The kind of a job, for the hooks.
pub const KIND: ResourceKind = ResourceKind::new("job");

/// The id of a job.
pub type JobId = Id<Job>;

/// The longest error text that a job or a node keeps, in characters.
pub const MAX_ERROR: usize = 4096;

/// The state of a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    /// The job waits for an agent, or for the start of its agent.
    Pending,
    /// The agent runs the job.
    Running,
    /// Each node completed or was skipped.
    Completed,
    /// A node failed, or the dispatch failed.
    Failed,
    /// A person cancelled the job.
    Cancelled,
    /// The agent of the running job went away.
    Orphaned,
}

impl JobStatus {
    /// Returns the name of the state, as the database keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Orphaned => "orphaned",
        }
    }

    /// Reads the name of a state.
    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::Pending,
            Self::Running,
            Self::Completed,
            Self::Failed,
            Self::Cancelled,
            Self::Orphaned,
        ]
        .into_iter()
        .find(|status| status.as_str() == name)
    }

    /// Returns `true` for a state that does not change again.
    pub fn is_final(self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
    }
}

/// The state of a node in a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeStatus {
    /// The node waits for its needs.
    Pending,
    /// The node runs.
    Running,
    /// The node completed.
    Completed,
    /// The node failed after its last attempt.
    Failed,
    /// The node did not run, because a node failed.
    Skipped,
    /// The job ended before the node ended.
    Cancelled,
}

impl NodeStatus {
    /// Returns the name of the state.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Cancelled => "cancelled",
        }
    }

    /// Reads the name of a state.
    pub fn parse(name: &str) -> Option<Self> {
        [
            Self::Pending,
            Self::Running,
            Self::Completed,
            Self::Failed,
            Self::Skipped,
            Self::Cancelled,
        ]
        .into_iter()
        .find(|status| status.as_str() == name)
    }

    /// Returns `true` for a state that does not change again.
    pub fn is_final(self) -> bool {
        !matches!(self, Self::Pending | Self::Running)
    }
}

/// A job, as the store returns it.
#[derive(Clone, Debug)]
pub struct Job {
    /// The id.
    pub id: JobId,
    /// The pipeline.
    pub pipeline_id: PipelineId,
    /// The project of the pipeline.
    pub project_id: ProjectId,
    /// The workspace of the project.
    pub workspace_id: WorkspaceId,
    /// The actor that started the job.
    pub created_by: ActorId,
    /// The job that this job runs again, if it is a rerun.
    pub rerun_of: Option<JobId>,
    /// The trigger that started the job, if a trigger did.
    pub trigger_id: Option<crate::projects::triggers::TriggerId>,
    /// The state.
    pub status: JobStatus,
    /// The nodes of the pipeline at the start of the job.
    pub nodes: Vec<Node>,
    /// The state of each node, in the order of the nodes.
    pub runs: Vec<NodeRun>,
    /// The agent of the job, once the server placed it.
    pub agent_id: Option<AppId>,
    /// The error of a failed job.
    pub error: Option<String>,
    /// The version: one more at each change.
    pub version: i64,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The start on the agent.
    pub started_at: Option<OffsetDateTime>,
    /// The end.
    pub finished_at: Option<OffsetDateTime>,
}

/// The input of a job creation: a run or a rerun.
#[derive(Clone, Debug)]
pub struct NewJob {
    /// The pipeline.
    pub pipeline_id: PipelineId,
    /// The project of the pipeline.
    pub project_id: ProjectId,
    /// The workspace of the project.
    pub workspace_id: WorkspaceId,
    /// The actor that starts the job.
    pub created_by: ActorId,
    /// The job that this job runs again.
    pub rerun_of: Option<JobId>,
    /// The trigger that starts the job.
    pub trigger_id: Option<crate::projects::triggers::TriggerId>,
    /// The nodes to run.
    pub nodes: Vec<Node>,
}

/// Where a job list looks.
#[derive(Clone, Copy, Debug)]
pub enum JobScope {
    /// The jobs of one pipeline.
    Pipeline(PipelineId),
    /// The jobs of a workspace, of one agent if it is set.
    Workspace(WorkspaceId, Option<AppId>),
}

/// The input of a job list: newest first.
#[derive(Clone, Copy, Debug)]
pub struct ListJobs {
    /// Where to look.
    pub scope: JobScope,
    /// The page to return: the jobs before the key of the page.
    pub page: PageRequest<JobId>,
}

/// One page of jobs.
pub type JobPage = Page<Job, JobId>;

/// The jobs that an agent may take.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RunScope {
    /// No job: the agent has no role `runner`.
    #[default]
    Nothing,
    /// The jobs of each project of its workspace.
    Workspace,
    /// The jobs of these projects only.
    Projects(Vec<ProjectId>),
}

/// Keeps at most `MAX_ERROR` characters of an error text.
pub fn error_text(text: &str) -> String {
    text.chars().take(MAX_ERROR).collect()
}
