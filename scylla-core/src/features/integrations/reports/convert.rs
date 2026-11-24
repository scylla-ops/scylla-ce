//! The proto values of the reports: a report of a job, and the reports of one job.

use scylla_domain::integrations::reports::{ReportMode, ReportState};
use scylla_domain::integrations::VcsProvider;
use scylla_proto::integrations::v1 as pb;

use super::service::JobReports;
use super::store::reads::Listed;
use crate::api::parse::{self, ToProto};

/// Makes the proto value of the reports of one job.
pub(super) fn job_reports(job: JobReports) -> pb::JobReports {
    pb::JobReports {
        job_id: Some(job.job.to_proto()),
        reports: job.reports.into_iter().map(report).collect(),
        no_report_reason: job.no_report.unwrap_or_default().to_owned(),
    }
}

/// Makes the proto value of a report. The time of the next call shows only for a report that
/// retries.
pub(super) fn report(listed: Listed) -> pb::VcsReport {
    let Listed { report, provider } = listed;
    let next_attempt_at = match report.state {
        ReportState::Retrying => report.next_attempt_at.map(parse::timestamp),
        _ => None,
    };
    pb::VcsReport {
        job_id: Some(report.job_id.to_proto()),
        provider: match provider {
            VcsProvider::GitHub => pb::VcsProvider::Github,
            VcsProvider::GitLab => pb::VcsProvider::Gitlab,
        } as i32,
        mode: match report.mode {
            ReportMode::CheckRun => pb::ReportMode::CheckRun,
            ReportMode::CommitStatus => pb::ReportMode::CommitStatus,
        } as i32,
        context: report.context,
        state: match report.state {
            ReportState::Sending => pb::ReportState::Sending,
            ReportState::Sent => pb::ReportState::Sent,
            ReportState::Retrying => pb::ReportState::Retrying,
            ReportState::NotSent => pb::ReportState::NotSent,
        } as i32,
        sent_state: report.sent_state.unwrap_or_default(),
        sent_at: report.sent_at.map(parse::timestamp),
        attempts: report.attempts,
        next_attempt_at,
        error: report.last_error.unwrap_or_default(),
        external_url: report.external_url.unwrap_or_default(),
    }
}
