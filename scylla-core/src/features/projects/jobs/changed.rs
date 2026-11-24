//! What follows each change of the state of a job, in the transaction of the change: the report
//! on its commit, and the row of the activity feed at its end.

use scylla_domain::projects::jobs::Job;
use scylla_extension::Tx;

use crate::features::activity;
use crate::features::integrations::reports;
use crate::Error;

/// Records the report of a job that changed and, if it ended, its row of the activity feed. Call it
/// after the write of the job.
pub(crate) async fn job_changed(tx: &mut Tx<'_>, job: &Job) -> Result<(), Error> {
    reports::job_changed(tx, job).await?;
    activity::job_ended(tx, job).await
}
