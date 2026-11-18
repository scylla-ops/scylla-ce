//! The pipelines feature: the `pipelines` table, `PipelineService`, the JSON of the nodes, the
//! check and the export of the pipeline files, the summaries of the dashboard, and the check of
//! the access to an object of a project.

pub(crate) mod access;
mod checks;
mod convert;
pub(crate) mod files_convert;
mod files_grpc;
mod files_service;
mod grpc;
pub(crate) mod json;
mod plugin;
pub(crate) mod service;
mod store;
mod summaries_convert;
mod summaries_grpc;
mod summaries_service;

pub(crate) use convert::{container, script};
pub(crate) use grpc::server;
pub(crate) use plugin::Pipelines;
