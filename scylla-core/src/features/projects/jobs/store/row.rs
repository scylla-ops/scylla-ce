//! A row of the `jobs` table, and the forms of its approval, pipeline file, file errors and
//! checkout in the columns.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::apps::AppId;
use scylla_domain::integrations::sources::{ApprovalState, JobSource};
use scylla_domain::projects::jobs::{
    CheckoutRun, Job, JobApproval, JobId, JobStatus, NodeStatus, PipelineFileRef,
};
use scylla_domain::projects::pipeline_files::{CheckoutSettings, FileError};
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::TriggerId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::features::projects::pipelines::json;
use crate::Error;

/// A row of the `jobs` table.
pub(super) struct Row {
    /// The id.
    pub(super) id: uuid::Uuid,
    /// The pipeline.
    pub(super) pipeline_id: uuid::Uuid,
    /// The project.
    pub(super) project_id: uuid::Uuid,
    /// The workspace.
    pub(super) workspace_id: uuid::Uuid,
    /// The author.
    pub(super) created_by: uuid::Uuid,
    /// The job that this job runs again.
    pub(super) rerun_of: Option<uuid::Uuid>,
    /// The trigger of the job.
    pub(super) trigger_id: Option<uuid::Uuid>,
    /// The state.
    pub(super) status: String,
    /// The nodes in JSON.
    pub(super) nodes: Value,
    /// The states of the nodes in JSON.
    pub(super) runs: Value,
    /// The agent.
    pub(super) agent_id: Option<uuid::Uuid>,
    /// The error.
    pub(super) error: Option<String>,
    /// The version.
    pub(super) version: i64,
    /// The creation time.
    pub(super) created_at: OffsetDateTime,
    /// The start.
    pub(super) started_at: Option<OffsetDateTime>,
    /// The end.
    pub(super) finished_at: Option<OffsetDateTime>,
    /// The state of the approval of a job from a fork.
    pub(super) approval: Option<String>,
    /// The actor that approved the job.
    pub(super) approved_by: Option<uuid::Uuid>,
    /// The time of the approval.
    pub(super) approved_at: Option<OffsetDateTime>,
    /// The actor that rejected the job.
    pub(super) rejected_by: Option<uuid::Uuid>,
    /// The time of the rejection.
    pub(super) rejected_at: Option<OffsetDateTime>,
    /// The checkout in JSON.
    pub(super) checkout: Option<Value>,
    /// The pipeline file in JSON.
    pub(super) pipeline_file: Option<Value>,
    /// The errors of the pipeline file in JSON.
    pub(super) file_errors: Option<Value>,
}

/// The columns of the approval of a job: the state, then the actor and the time of an approval
/// and of a rejection.
pub(super) type ApprovalColumns<'a> = (
    Option<&'a str>,
    Option<uuid::Uuid>,
    Option<OffsetDateTime>,
    Option<uuid::Uuid>,
    Option<OffsetDateTime>,
);

/// The checkout in JSON, with its settings. The times are unix microseconds.
#[derive(Deserialize, Serialize)]
struct CheckoutJson {
    /// The state.
    status: String,
    /// The start.
    started_at: Option<i64>,
    /// The end.
    finished_at: Option<i64>,
    /// The error.
    error: Option<String>,
    /// The number of commits to fetch.
    #[serde(default = "one")]
    depth: u32,
    /// `true` also fetches the submodules.
    #[serde(default)]
    submodules: bool,
    /// `true` also fetches the LFS files.
    #[serde(default)]
    lfs: bool,
}

impl Row {
    /// Makes the job of the row, with its source.
    pub(super) fn into_job(self, source: Option<JobSource>) -> Result<Job, Error> {
        let approval = self.approval()?;
        Ok(Job {
            id: JobId::from_db(self.id),
            pipeline_id: PipelineId::from_db(self.pipeline_id),
            project_id: ProjectId::from_db(self.project_id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            created_by: ActorId::from_db(self.created_by),
            rerun_of: self.rerun_of.map(JobId::from_db),
            trigger_id: self.trigger_id.map(TriggerId::from_db),
            status: JobStatus::parse(&self.status).ok_or_else(|| bad_status(&self.status))?,
            nodes: json::read_nodes(self.nodes)?,
            runs: json::read_runs(self.runs)?,
            agent_id: self.agent_id.map(AppId::from_db),
            error: self.error,
            version: self.version,
            created_at: self.created_at,
            started_at: self.started_at,
            finished_at: self.finished_at,
            source,
            approval,
            pipeline_file: self.pipeline_file.map(read).transpose()?,
            file_errors: self.file_errors.map(read).transpose()?.unwrap_or_default(),
            checkout: self.checkout.map(read_checkout).transpose()?,
        })
    }

    /// Reads the approval of the row.
    fn approval(&self) -> Result<Option<JobApproval>, Error> {
        let Some(state) = &self.approval else {
            return Ok(None);
        };
        let state = ApprovalState::parse(state).ok_or_else(|| bad("approval"))?;
        let (by, at) = match state {
            ApprovalState::Waiting => (None, None),
            ApprovalState::Approved => (self.approved_by, self.approved_at),
            ApprovalState::Rejected => (self.rejected_by, self.rejected_at),
        };
        Ok(Some(JobApproval {
            state,
            decided_by: by.map(ActorId::from_db),
            decided_at: at,
        }))
    }
}

/// Returns the columns of the approval of a job.
pub(super) fn approval_columns(approval: Option<&JobApproval>) -> ApprovalColumns<'_> {
    let Some(approval) = approval else {
        return (None, None, None, None, None);
    };
    let by = approval.decided_by.map(ActorId::to_db);
    let state = Some(approval.state.as_str());
    match approval.state {
        ApprovalState::Waiting => (state, None, None, None, None),
        ApprovalState::Approved => (state, by, approval.decided_at, None, None),
        ApprovalState::Rejected => (state, None, None, by, approval.decided_at),
    }
}

/// Returns the JSON of a pipeline file.
pub(super) fn pipeline_file(file: Option<&PipelineFileRef>) -> Option<Value> {
    file.and_then(|file| serde_json::to_value(file).ok())
}

/// Returns the JSON of the errors of a pipeline file, or `None` for no error.
pub(super) fn file_errors(errors: &[FileError]) -> Option<Value> {
    match errors.is_empty() {
        true => None,
        false => serde_json::to_value(errors).ok(),
    }
}

/// Returns the JSON of the checkout of a job.
pub(super) fn checkout(run: Option<&CheckoutRun>) -> Option<Value> {
    let micros = |at: OffsetDateTime| (at.unix_timestamp_nanos() / 1_000) as i64;
    let json = run.map(|run| CheckoutJson {
        status: run.status.as_str().to_owned(),
        started_at: run.started_at.map(micros),
        finished_at: run.finished_at.map(micros),
        error: run.error.clone(),
        depth: run.settings.depth,
        submodules: run.settings.submodules,
        lfs: run.settings.lfs,
    })?;
    serde_json::to_value(json).ok()
}

/// Reads a value from its JSON.
fn read<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(bad)
}

/// Reads a checkout from its JSON.
fn read_checkout(value: Value) -> Result<CheckoutRun, Error> {
    let json: CheckoutJson = read(value)?;
    let time = |micros: i64| {
        OffsetDateTime::from_unix_timestamp_nanos(i128::from(micros) * 1_000).map_err(bad)
    };
    Ok(CheckoutRun {
        status: NodeStatus::parse(&json.status).ok_or_else(|| bad("checkout status"))?,
        started_at: json.started_at.map(time).transpose()?,
        finished_at: json.finished_at.map(time).transpose()?,
        error: json.error,
        settings: CheckoutSettings {
            enabled: true,
            depth: json.depth,
            submodules: json.submodules,
            lfs: json.lfs,
        },
    })
}

/// The depth of a checkout with no depth in its JSON.
fn one() -> u32 {
    1
}

/// The error of a state that the code does not know.
pub(super) fn bad_status(status: &str) -> Error {
    Error::Internal(format!("unknown job status {status}"))
}

/// The error of bad data of a job in the database.
fn bad(error: impl std::fmt::Display) -> Error {
    Error::Internal(format!("bad job data in the database: {error}"))
}
