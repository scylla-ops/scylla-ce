//! The pipelines of a project: a graph of nodes that a job runs on an agent.

mod graph;
mod layout;
mod nodes;
#[cfg(test)]
mod tests;

pub use graph::Nodes;
pub use layout::{PipelineLayout, Position};
pub use nodes::{env_name, EnvValue, EnvVar, Node, NodeError, NodeId, Runtime, Shell, Step};
use time::OffsetDateTime;

use crate::identity::actors::{Actor, ActorId};
use crate::projects::{Project, ProjectId};
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::{Id, Name, Page, PageRequest, ResourceKind};

/// The kind of a pipeline, for the hooks.
pub const KIND: ResourceKind = ResourceKind::new("pipeline");

/// The id of a pipeline.
pub type PipelineId = Id<Pipeline>;

/// The display name of a pipeline.
pub type PipelineName = Name;

/// A pipeline, as the store returns it.
#[derive(Clone, Debug)]
pub struct Pipeline {
    /// The id.
    pub id: PipelineId,
    /// The project of the pipeline.
    pub project_id: ProjectId,
    /// The workspace of the project.
    pub workspace_id: WorkspaceId,
    /// The actor that created the pipeline.
    pub created_by: ActorId,
    /// The display name.
    pub name: String,
    /// The nodes.
    pub nodes: Vec<Node>,
    /// The version: 1 at the creation, and one more at each change.
    pub version: i64,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The time of the last change.
    pub updated_at: OffsetDateTime,
}

/// The input of a pipeline creation.
#[derive(Clone, Debug)]
pub struct NewPipeline {
    /// The project.
    pub project_id: ProjectId,
    /// The actor that creates the pipeline.
    pub created_by: ActorId,
    /// The display name.
    pub name: PipelineName,
    /// The checked nodes.
    pub nodes: Nodes,
    /// The places of the new nodes.
    pub layout: PipelineLayout,
}

/// A change of a pipeline. A field that is `None` does not change. The change passes only if
/// the pipeline still has the version that the client read.
#[derive(Clone, Debug)]
pub struct PipelineChange {
    /// The version that the client read.
    pub version: i64,
    /// The new display name.
    pub name: Option<PipelineName>,
    /// The new nodes: they replace all the nodes.
    pub nodes: Option<Nodes>,
    /// The places to save with the change. An empty layout saves nothing.
    pub layout: PipelineLayout,
}

/// Where a pipeline list looks.
#[derive(Clone, Copy, Debug)]
pub enum PipelineScope {
    /// The pipelines of one project.
    Project(ProjectId),
    /// The pipelines of each project of a workspace.
    Workspace(WorkspaceId),
}

/// The input of a pipeline list.
#[derive(Clone, Copy, Debug)]
pub struct ListPipelines {
    /// Where to look.
    pub scope: PipelineScope,
    /// The page to return, oldest first.
    pub page: PageRequest<PipelineId>,
}

/// One page of pipelines.
pub type PipelinePage = Page<Pipeline, PipelineId>;

crate::authz! {
    /// The routes of the pipelines and of their jobs on a project, and the role of the agents of
    /// this project only.
    extend Project as PipelineAccess {
        /// An agent that runs the jobs of this project only.
        relation runner: Actor;
        /// The agents get the jobs of this project. It is not a route.
        permission run_jobs = runner;
        /// The route `CreatePipeline`, in this project.
        permission create_pipeline = edit;
        /// The route `GetPipeline`, on each pipeline of this project.
        permission get_pipeline = view;
        /// The route `UpdatePipeline`, on each pipeline of this project.
        permission update_pipeline = edit;
        /// The route `SavePipelineLayout`, on each pipeline of this project.
        permission save_pipeline_layout = edit;
        /// The route `DeletePipeline`, on each pipeline of this project.
        permission delete_pipeline = edit;
        /// The route `ListProjectPipelines`: the pipelines of this project.
        permission list_project_pipelines = view;
        /// The route `RunPipeline`, on each pipeline of this project.
        permission run_pipeline = edit;
        /// The route `GetJob`, on each job of this project.
        permission get_job = view;
        /// The route `CancelJob`, on each job of this project.
        permission cancel_job = edit;
        /// The route `DeleteJob`, on each job of this project.
        permission delete_job = edit;
        /// The route `RerunJob`, on each job of this project.
        permission rerun_job = edit;
        /// The route `ListPipelineJobs`, on each pipeline of this project.
        permission list_pipeline_jobs = view;
        /// The route `TailJobLogs`, on each job of this project.
        permission tail_job_logs = view;
        /// The route `SetPipelineSource`, on this project.
        permission set_pipeline_source = edit;
        /// The route `ApproveJob`, on each job of this project.
        permission approve_job = edit;
        /// The route `RejectJob`, on each job of this project.
        permission reject_job = edit;
        /// The route `ListJobReports`, on each job of this project.
        permission list_job_reports = view;
        /// The route `RetryJobReport`, on each job of this project.
        permission retry_job_report = edit;
        /// The route `ValidatePipelineFile`, in this project.
        permission validate_pipeline_file = view;
        /// The route `ExportPipeline`, on each pipeline of this project.
        permission export_pipeline = view;
        /// The route `GetFilePipeline`, on each pipeline of this project.
        permission get_file_pipeline = view;
        /// The route `ListPipelineFiles`, in this project.
        permission list_pipeline_files = view;
    }

    /// The routes of the pipelines and of their jobs on a workspace, and the role of the agents.
    extend Workspace as PipelineHolder {
        /// An agent that runs the jobs of this workspace.
        relation runner: Actor;
        /// The agents get the jobs of this workspace. It is not a route.
        permission run_jobs = runner;
        /// The route `CreatePipeline`, in each project of this workspace.
        permission create_pipeline = work;
        /// The route `GetPipeline`, in each project of this workspace.
        permission get_pipeline = read;
        /// The route `UpdatePipeline`, in each project of this workspace.
        permission update_pipeline = work;
        /// The route `SavePipelineLayout`, in each project of this workspace.
        permission save_pipeline_layout = work;
        /// The route `DeletePipeline`, in each project of this workspace.
        permission delete_pipeline = work;
        /// The route `ListProjectPipelines`, for each project of this workspace.
        permission list_project_pipelines = read;
        /// The route `ListWorkspacePipelines`: the pipelines of this workspace.
        permission list_workspace_pipelines = read;
        /// The route `RunPipeline`, in each project of this workspace.
        permission run_pipeline = work;
        /// The route `GetJob`, in each project of this workspace.
        permission get_job = read;
        /// The route `CancelJob`, in each project of this workspace.
        permission cancel_job = work;
        /// The route `DeleteJob`, in each project of this workspace.
        permission delete_job = work;
        /// The route `RerunJob`, in each project of this workspace.
        permission rerun_job = work;
        /// The route `ListPipelineJobs`, in each project of this workspace.
        permission list_pipeline_jobs = read;
        /// The route `ListWorkspaceJobs`: the jobs of this workspace.
        permission list_workspace_jobs = read;
        /// The route `TailJobLogs`, in each project of this workspace.
        permission tail_job_logs = read;
        /// The route `SetPipelineSource`, in each project of this workspace.
        permission set_pipeline_source = work;
        /// The route `ApproveJob`, in each project of this workspace.
        permission approve_job = work;
        /// The route `RejectJob`, in each project of this workspace.
        permission reject_job = work;
        /// The route `ListJobReports`, in each project of this workspace.
        permission list_job_reports = read;
        /// The route `RetryJobReport`, in each project of this workspace.
        permission retry_job_report = work;
        /// The route `ValidatePipelineFile`, in each project of this workspace.
        permission validate_pipeline_file = read;
        /// The route `ExportPipeline`, in each project of this workspace.
        permission export_pipeline = read;
        /// The route `GetFilePipeline`, in each project of this workspace.
        permission get_file_pipeline = read;
        /// The route `ListPipelineFiles`, in each project of this workspace.
        permission list_pipeline_files = read;
    }
}
