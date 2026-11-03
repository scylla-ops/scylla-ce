//! The projects feature: the `projects` table and `ProjectService`. The secrets, the pipelines,
//! the jobs and the triggers of the projects are in `secrets`, `pipelines`, `jobs` and
//! `triggers`.

mod grpc;
pub(crate) mod jobs;
pub(crate) mod pipelines;
mod plugin;
pub(crate) mod secrets;
mod service;
mod store;
pub(crate) mod triggers;

#[cfg(test)]
mod tests;

pub(crate) use plugin::Projects;
