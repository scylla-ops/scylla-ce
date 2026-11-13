//! The reports of the jobs to the provider: one check run or commit status for each pipeline, on
//! the commit of the job, through an outbox.

use time::OffsetDateTime;

use super::connections::VcsConnectionId;
use super::links::ReportMode;
use super::named;
use crate::projects::jobs::{JobId, JobStatus};
use crate::projects::ProjectId;

/// The most characters of the key of a pipeline.
pub const MAX_KEY: usize = 64;

/// The key of a pipeline whose name gives no letter and no digit.
const FALLBACK_KEY: &str = "pipeline";

named! {
    /// The delivery state of a report.
    pub enum ReportState {
        /// The latest state waits for its first call.
        Sending = "sending",
        /// The provider got the latest state.
        Sent = "sent",
        /// A call failed, and the sender tries again later.
        Retrying = "retrying",
        /// The sender stopped: too many failures, or a connection that needs a person.
        NotSent = "not_sent",
    }
}

named! {
    /// The state of a run that a report tells the provider.
    pub enum ReportedStatus {
        /// The run from a fork waits for an approval.
        AwaitingApproval = "awaiting_approval",
        /// The run waits for an agent.
        Pending = "pending",
        /// The run is on an agent.
        Running = "running",
        /// Each node completed or was skipped.
        Completed = "completed",
        /// A node failed, or the dispatch failed.
        Failed = "failed",
        /// A person cancelled or rejected the run.
        Cancelled = "cancelled",
        /// The agent of the run went away.
        Orphaned = "orphaned",
        /// The pipeline file of the commit is not valid, so no node ran.
        InvalidFile = "invalid_file",
    }
}

impl ReportedStatus {
    /// Returns the state that a report tells for a job: `InvalidFile` for a failed job with
    /// errors of its pipeline file, else the state of the job.
    pub fn of_job(status: JobStatus, invalid_file: bool) -> Self {
        match status {
            JobStatus::Failed if invalid_file => Self::InvalidFile,
            JobStatus::AwaitingApproval => Self::AwaitingApproval,
            JobStatus::Pending => Self::Pending,
            JobStatus::Running => Self::Running,
            JobStatus::Completed => Self::Completed,
            JobStatus::Failed => Self::Failed,
            JobStatus::Cancelled => Self::Cancelled,
            JobStatus::Orphaned => Self::Orphaned,
        }
    }

    /// Returns `true` for a state that does not change again. The sender never sends a state
    /// that is not final after a final one.
    pub fn is_final(self) -> bool {
        !matches!(self, Self::AwaitingApproval | Self::Pending | Self::Running)
    }

    /// Returns `true` when a report that wants this state takes the state `next`: a new state,
    /// and never a state that is not final after a final one.
    pub fn takes(self, next: Self) -> bool {
        self != next && (!self.is_final() || next.is_final())
    }
}

/// The report of one pipeline of a job, as the store returns it.
#[derive(Clone, Debug)]
pub struct VcsReport {
    /// The job.
    pub job_id: JobId,
    /// The name of the report, for example `scylla/build`.
    pub context: String,
    /// The project of the job.
    pub project_id: ProjectId,
    /// The connection that sends the report.
    pub connection_id: VcsConnectionId,
    /// A check run or a commit status.
    pub mode: ReportMode,
    /// The latest state of the run.
    pub wanted: ReportedStatus,
    /// The number of the latest state: one more at each change of the run.
    pub sequence: i64,
    /// The delivery state.
    pub state: ReportState,
    /// The last state that the provider got, in the words of the provider.
    pub sent_state: Option<String>,
    /// The time of the last send.
    pub sent_at: Option<OffsetDateTime>,
    /// The id of the check run on the provider.
    pub external_id: Option<String>,
    /// The page of the report on the provider.
    pub external_url: Option<String>,
    /// The number of failed calls for the latest state.
    pub attempts: u32,
    /// The time of the next call.
    pub next_attempt_at: Option<OffsetDateTime>,
    /// The last error, as a short text with no secret.
    pub last_error: Option<String>,
}

/// Returns the name of the report of a pipeline: `scylla/` and its key.
pub fn report_context(key: &str) -> String {
    format!("scylla/{key}")
}

/// Returns the key of a pipeline of the editor, from its name: ASCII letters in lower case,
/// digits, and one dash for each run of other characters, cut to `MAX_KEY`. A name with no
/// letter and no digit gives `pipeline`.
pub fn pipeline_key(name: &str) -> String {
    // Keep the letters and the digits, with one dash between two runs.
    let ascii = deunicode::deunicode(name).to_ascii_lowercase();
    let mut key = String::with_capacity(ascii.len());
    for c in ascii.chars() {
        if c.is_ascii_alphanumeric() {
            key.push(c);
        } else if !key.is_empty() && !key.ends_with('-') {
            key.push('-');
        }
    }

    // Cut it to the maximum length, with no dash at the end.
    let key = key[..key.len().min(MAX_KEY)].trim_end_matches('-');
    if key.is_empty() {
        return FALLBACK_KEY.to_owned();
    }
    key.to_owned()
}
