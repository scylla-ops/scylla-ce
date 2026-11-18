//! The jobs feature: the `jobs` and `job_logs` tables, `JobService` with the figures of the
//! jobs, the jobs of the agents on their stream with their busy slots, and the background task of
//! the lost agents, of the old logs and of the old samples of the agents.

mod approval;
pub(crate) mod commits;
pub(crate) mod convert;
mod env;
mod figures;
mod figures_convert;
mod figures_grpc;
mod filter;
mod grpc;
pub(crate) mod housekeeping;
mod lists;
pub(crate) mod runs;
mod service;
pub(crate) mod slots;
pub(crate) mod stats;
mod store;
mod tail;

#[cfg(test)]
pub(crate) mod tests;

pub(crate) use grpc::server;
pub(crate) use service::{create, get, rerun};
