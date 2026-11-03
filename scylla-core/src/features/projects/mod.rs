//! The projects feature: the `projects` table and `ProjectService`. The secrets, the pipelines
//! and the jobs of the projects are in `secrets`, `pipelines` and `jobs`.

mod grpc;
pub(crate) mod jobs;
pub(crate) mod pipelines;
mod plugin;
pub(crate) mod secrets;
mod service;
mod store;

#[cfg(test)]
mod tests;

pub(crate) use plugin::Projects;
