//! The reports of the runs on the commits: statuses and check runs.

mod mount;
mod outbox;

pub(crate) use mount::mount;
#[allow(unused_imports, reason = "each change of the status of a job calls it")]
pub(crate) use outbox::job_changed;
