//! The pipelines feature: the `pipelines` table, `PipelineService`, the JSON of the nodes, and
//! the check of the access to an object of a project.

pub(crate) mod access;
mod checks;
mod convert;
mod files_grpc;
mod grpc;
pub(crate) mod json;
mod plugin;
pub(crate) mod service;
mod store;

pub(crate) use convert::{container, script};
pub(crate) use grpc::server;
pub(crate) use plugin::Pipelines;
