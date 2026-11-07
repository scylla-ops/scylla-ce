//! The pipelines feature: the `pipelines` table, `PipelineService`, the JSON of the nodes, the
//! conversions of the needs and of the switches, and the check of the access to an object of a
//! project.

pub(crate) mod access;
mod convert;
mod grpc;
pub(crate) mod json;
mod plugin;
pub(crate) mod service;
mod store;
mod switch;

pub(crate) use convert::{container, script};
pub(crate) use grpc::server;
pub(crate) use plugin::Pipelines;
pub(crate) use switch::{needs, switch};
