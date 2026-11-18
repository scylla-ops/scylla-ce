//! The input of a job list: where it looks, its filters and its page.

use time::OffsetDateTime;

use super::{Job, JobId, JobOrigin, JobStatus};
use crate::identity::apps::AppId;
use crate::integrations::sources::SourceEvent;
use crate::projects::pipelines::PipelineId;
use crate::projects::ProjectId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Page, PageRequest};

/// The most project ids or pipeline ids in a job list.
pub const MAX_LIST_IDS: usize = 100;

/// Where a job list looks.
#[derive(Clone, Debug)]
pub enum JobScope {
    /// The jobs of one pipeline.
    Pipeline(PipelineId),
    /// The jobs of a workspace that the caller can see.
    Workspace(WorkspaceJobs),
}

/// The jobs of a workspace in a list: the projects that the caller can see, and the filters of
/// the request.
#[derive(Clone, Debug)]
pub struct WorkspaceJobs {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The projects that the caller can see, or `None` for each project of the workspace.
    pub visible: Option<Vec<ProjectId>>,
    /// Only the jobs of this agent.
    pub agent: Option<AppId>,
    /// Only the jobs of these projects. Empty gives each project.
    pub projects: Vec<ProjectId>,
    /// Only the jobs of these pipelines. Empty gives each pipeline.
    pub pipelines: Vec<PipelineId>,
}

/// The filters of a job list. A job must match each filter that is set, and a job with no
/// source matches no filter of the source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JobFilter {
    /// Only the jobs of this branch.
    pub branch: Option<String>,
    /// Only the jobs of this event.
    pub event: Option<SourceEvent>,
    /// Only the jobs of the commits of this author login.
    pub author_login: Option<String>,
    /// Only the jobs of this pull request number.
    pub pull_request: Option<u32>,
    /// Only the jobs in one of these states. Empty gives each state.
    pub statuses: Vec<JobStatus>,
    /// Only the jobs created at this time or later.
    pub created_after: Option<OffsetDateTime>,
    /// Only the jobs created before this time.
    pub created_before: Option<OffsetDateTime>,
    /// Only the jobs with one of these origins. Empty gives each origin.
    pub origins: Vec<JobOrigin>,
}

/// The input of a job list: newest first.
#[derive(Clone, Debug)]
pub struct ListJobs {
    /// Where to look.
    pub scope: JobScope,
    /// The page to return: the jobs before the key of the page.
    pub page: PageRequest<JobId>,
    /// The filters.
    pub filter: JobFilter,
}

/// One page of jobs.
pub type JobPage = Page<Job, JobId>;
