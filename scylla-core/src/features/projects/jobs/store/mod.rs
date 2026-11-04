//! The SQL of the jobs: the `jobs` and `job_logs` tables, and the jobs that the agents hold. Each function runs in the transaction
//! of the caller.

pub(crate) mod holds;
pub(crate) mod jobs;
pub(crate) mod logs;
pub(crate) mod stats;
