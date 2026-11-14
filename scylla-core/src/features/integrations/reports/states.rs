//! The map from the state of a run to the state of a commit status on GitHub and on GitLab, to
//! the status and the conclusion of a GitHub check run, and the short description of the report.

use scylla_domain::integrations::reports::ReportedStatus;
use scylla_domain::integrations::VcsProvider;
use time::OffsetDateTime;

/// The most characters of a description on GitHub.
const GITHUB_DESCRIPTION: usize = 140;

/// The most characters of a description on GitLab.
const GITLAB_DESCRIPTION: usize = 255;

/// What the description of a report tells about its job.
#[derive(Default)]
pub(super) struct Run<'a> {
    /// The start of the job on the agent.
    pub(super) started_at: Option<OffsetDateTime>,
    /// The end of the job.
    pub(super) finished_at: Option<OffsetDateTime>,
    /// The first node that failed.
    pub(super) failed_node: Option<&'a str>,
    /// The path of the pipeline file of the job.
    pub(super) file_path: Option<&'a str>,
    /// The line of the first error of an invalid pipeline file.
    pub(super) file_line: Option<u32>,
}

/// Returns the state of a GitHub commit status: `pending`, `success`, `failure` or `error`.
pub(super) fn github(status: ReportedStatus) -> &'static str {
    match status {
        ReportedStatus::AwaitingApproval | ReportedStatus::Pending | ReportedStatus::Running => {
            "pending"
        }
        ReportedStatus::Completed => "success",
        ReportedStatus::Failed | ReportedStatus::InvalidFile => "failure",
        ReportedStatus::Cancelled | ReportedStatus::Orphaned => "error",
    }
}

/// Returns the status of a GitHub check run (`queued`, `in_progress` or `completed`), and the
/// conclusion of a completed one (`success`, `failure` or `cancelled`).
pub(super) fn check_run(status: ReportedStatus) -> (&'static str, Option<&'static str>) {
    match status {
        ReportedStatus::AwaitingApproval | ReportedStatus::Pending => ("queued", None),
        ReportedStatus::Running => ("in_progress", None),
        ReportedStatus::Completed => ("completed", Some("success")),
        ReportedStatus::Failed | ReportedStatus::Orphaned | ReportedStatus::InvalidFile => {
            ("completed", Some("failure"))
        }
        ReportedStatus::Cancelled => ("completed", Some("cancelled")),
    }
}

/// Returns the state of a GitLab commit status: `pending`, `running`, `success`, `failed` or
/// `canceled`.
pub(super) fn gitlab(status: ReportedStatus) -> &'static str {
    match status {
        ReportedStatus::AwaitingApproval | ReportedStatus::Pending => "pending",
        ReportedStatus::Running => "running",
        ReportedStatus::Completed => "success",
        ReportedStatus::Failed | ReportedStatus::Orphaned | ReportedStatus::InvalidFile => "failed",
        ReportedStatus::Cancelled => "canceled",
    }
}

/// Returns the description of a report on a provider, cut to the limit of the provider.
pub(super) fn description(provider: VcsProvider, status: ReportedStatus, run: &Run<'_>) -> String {
    // Tell the state, with the duration or the cause.
    let duration = match (run.started_at, run.finished_at) {
        (Some(start), Some(end)) => Some(length(end - start)),
        _ => None,
    };
    let text = match status {
        ReportedStatus::AwaitingApproval => "Waiting for approval in Scylla".to_owned(),
        ReportedStatus::Pending => "Queued".to_owned(),
        ReportedStatus::Running => "Running".to_owned(),
        ReportedStatus::Completed => match duration {
            Some(duration) => format!("Passed in {duration}"),
            None => "Passed".to_owned(),
        },
        ReportedStatus::Failed => match run.failed_node {
            Some(node) => format!("Failed: node {node}"),
            None => "Failed".to_owned(),
        },
        ReportedStatus::Cancelled => "Cancelled".to_owned(),
        ReportedStatus::Orphaned => "Agent lost".to_owned(),
        ReportedStatus::InvalidFile => match (run.file_path, run.file_line) {
            (Some(path), Some(line)) => format!("Invalid {path}: line {line}"),
            (Some(path), None) => format!("Invalid {path}"),
            (None, _) => "Invalid pipeline file".to_owned(),
        },
    };

    // Cut it to the limit of the provider.
    let limit = match provider {
        VcsProvider::GitHub => GITHUB_DESCRIPTION,
        VcsProvider::GitLab => GITLAB_DESCRIPTION,
    };
    text.chars().take(limit).collect()
}

/// Returns a duration as `58s`, `2m 14s` or `1h 3m`.
pub(super) fn length(duration: time::Duration) -> String {
    let seconds = duration.whole_seconds().max(0);
    match seconds {
        0..=59 => format!("{seconds}s"),
        60..=3599 => format!("{}m {}s", seconds / 60, seconds % 60),
        _ => format!("{}h {}m", seconds / 3600, seconds % 3600 / 60),
    }
}
