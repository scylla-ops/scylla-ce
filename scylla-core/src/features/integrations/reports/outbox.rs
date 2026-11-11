//! The outbox of the reports: a change of a job records the state that its reports must send.

use scylla_domain::projects::jobs::Job;
use scylla_extension::Tx;

use crate::Error;

/// Records the new state of a job for its reports, in the transaction of the change. It records
/// nothing yet.
#[allow(dead_code, reason = "each change of the status of a job calls it")]
pub(crate) async fn job_changed(_tx: &mut Tx<'_>, _job: &Job) -> Result<(), Error> {
    Ok(())
}
