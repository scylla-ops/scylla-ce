//! The logic of `VcsReportService`: the place of the jobs, their reports with the reason of a job
//! with none, and a report that a person sends again. One transaction for each call.

use std::collections::HashMap;

use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;

use super::store::reads::{self, Listed, Why};
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// The reports of one job.
pub(super) struct JobReports {
    /// The job.
    pub(super) job: JobId,
    /// Its reports, by name.
    pub(super) reports: Vec<Listed>,
    /// Why the job has no report, as a stable code, or `None` when it has reports.
    pub(super) no_report: Option<&'static str>,
}

/// Returns the project and the workspace of each known job of a list.
pub(super) async fn places(
    ctx: &IntegrationContext,
    jobs: &[JobId],
) -> Result<Vec<(JobId, ProjectId, WorkspaceId)>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let places = reads::places(&mut tx, jobs).await?;
    tx.commit().await?;
    Ok(places)
}

/// Returns the reports of each job of a list, in the order of the list.
pub(super) async fn list(
    ctx: &IntegrationContext,
    jobs: &[JobId],
) -> Result<Vec<JobReports>, Error> {
    // Read the reports, and what tells why a job has none.
    let mut tx = ctx.pool.begin().await?;
    let listed = reads::list(&mut tx, jobs).await?;
    let why = reads::why(&mut tx, jobs).await?;
    tx.commit().await?;

    // Give each job its reports, or the reason.
    let mut by_job: HashMap<JobId, Vec<Listed>> = HashMap::new();
    for item in listed {
        by_job.entry(item.report.job_id).or_default().push(item);
    }
    Ok(jobs
        .iter()
        .map(|job| {
            let reports = by_job.remove(job).unwrap_or_default();
            let no_report = match reports.is_empty() {
                true => Some(reason(why.iter().find(|item| item.job == *job))),
                false => None,
            };
            JobReports {
                job: *job,
                reports,
                no_report,
            }
        })
        .collect())
}

/// Makes a report that retries or that stopped due at once, and wakes the sender. Returns the
/// report, or `NOT_FOUND` for an unknown name.
pub(super) async fn retry(
    ctx: &IntegrationContext,
    job: JobId,
    context: &str,
) -> Result<Listed, Error> {
    let mut tx = ctx.pool.begin().await?;
    let listed = reads::retry(&mut tx, job, context).await?;
    tx.commit().await?;
    let listed = listed.ok_or(Error::NotFound("report"))?;
    ctx.wake.notify_one();
    Ok(listed)
}

/// Returns why a job has no report: no commit, no link, the reports off, or a link or a
/// connection that is not active.
fn reason(why: Option<&Why>) -> &'static str {
    match why {
        Some(why) if !why.source => "NO_SOURCE",
        Some(why) if !why.linked => "NO_REPOSITORY_LINK",
        Some(why) if why.reports && !why.active => "REPOSITORY_NOT_ACTIVE",
        _ => "REPORTS_OFF",
    }
}
