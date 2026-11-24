//! The SQL of the jobs: the `jobs`, `job_sources` and `job_logs` tables, the jobs that the
//! agents hold, and their busy slots. Each function runs in the transaction of the caller.

pub(crate) mod approvals;
pub(crate) mod figures;
pub(crate) mod holds;
pub(crate) mod jobs;
pub(crate) mod lists;
pub(crate) mod logs;
mod row;
pub(crate) mod slots;
pub(crate) mod sources;
pub(crate) mod stats;
