//! The groups of the projects of a workspace and the order of the projects: the
//! `project_groups` table, the places in `projects`, and `ProjectGroupService`.

mod grpc;
mod plugin;
mod service;
mod store;

pub(crate) use plugin::ProjectGroups;
