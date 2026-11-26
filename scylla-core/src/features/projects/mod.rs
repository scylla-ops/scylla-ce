//! The projects feature: the `projects` table and `ProjectService`. The groups, the secrets, the
//! pipelines, the jobs and the triggers of the projects are in `groups`, `secrets`, `pipelines`,
//! `jobs` and `triggers`.

mod convert;
pub(crate) mod groups;
mod grpc;
pub(crate) mod jobs;
pub(crate) mod pipelines;
mod plugin;
pub(crate) mod secrets;
mod service;
mod store;
pub(crate) mod triggers;

#[cfg(test)]
mod no_gate;
#[cfg(test)]
mod tests;

pub(crate) use plugin::Projects;
