//! The jobs feature: the `jobs` and `job_logs` tables, `JobService`, the jobs of the agents on
//! their stream, and the background task of the lost agents and of the old logs.

mod approval;
mod convert;
mod env;
mod grpc;
pub(crate) mod housekeeping;
pub(crate) mod runs;
mod service;
pub(crate) mod stats;
mod store;
mod tail;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) use grpc::server;
pub(crate) use service::{create, get, rerun};
