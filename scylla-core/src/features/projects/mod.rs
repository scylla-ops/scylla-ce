//! The projects feature: the `projects` table and `ProjectService`. The secrets of the projects
//! are in `secrets`.

mod grpc;
mod plugin;
pub(crate) mod secrets;
mod service;
mod store;

#[cfg(test)]
mod tests;

pub(crate) use plugin::Projects;
