//! The outbox of the reports: a change of a job records the state that its report must send, in
//! the transaction of the change. The sender posts it later.

use scylla_domain::integrations::reports::{
    pipeline_key, report_context, ReportMode, ReportedStatus,
};
use scylla_domain::projects::jobs::Job;
use scylla_extension::Tx;

use super::store::outbox::{self, Wanted};
use crate::Error;

/// Records the new state of a job for its report, in the transaction of the change: only for a
/// job with a commit, in a project whose link reports. While the link or its connection is not
/// active, a report keeps the new state as not sent, so a retry after the repair sends it.
pub(crate) async fn job_changed(tx: &mut Tx<'_>, job: &Job) -> Result<(), Error> {
    // Skip a job with no commit, and a project with no link that reports.
    if job.source.is_none() {
        return Ok(());
    }
    let Some(target) = outbox::target(tx, job.project_id, job.pipeline_id).await? else {
        return Ok(());
    };
    if !target.reports {
        return Ok(());
    }

    // Record the state of the report of the pipeline.
    let key = target
        .file_key
        .unwrap_or_else(|| pipeline_key(&target.name));
    let wanted = Wanted {
        job: job.id,
        context: report_context(&key),
        project: job.project_id,
        connection: target.connection,
        mode: ReportMode::of(target.provider),
        status: ReportedStatus::of_job(job.status, !job.file_errors.is_empty()),
        active: target.active,
    };
    outbox::record(tx, &wanted).await
}
