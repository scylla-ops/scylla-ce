//! The pipelines feature: the `pipelines` table, `PipelineService`, the JSON of the nodes, the
//! conversions of the needs and of the switches, the check and the export of the pipeline files,
//! the summaries of the dashboard, the keys that the file pipelines and the editor pipelines
//! share, the opt-in of the Git events, and the check of the access to an object of a project.

pub(crate) mod access;
mod checks;
pub(crate) mod convert;
pub(crate) mod files_convert;
mod files_grpc;
mod files_service;
mod git_events_grpc;
mod git_events_service;
mod grpc;
pub(crate) mod json;
pub(crate) mod keys;
mod links;
mod lists_grpc;
mod plugin;
pub(crate) mod service;
mod store;
mod summaries_convert;
mod summaries_grpc;
mod summaries_service;
mod switch;

pub(crate) use convert::{container, script};
pub(crate) use grpc::server;
pub(crate) use plugin::Pipelines;
pub(crate) use switch::{needs, switch};
