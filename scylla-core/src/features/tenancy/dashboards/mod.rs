//! The saved layouts of the dashboard of a workspace: the `dashboards` table and
//! `DashboardService`, with the open items that need a person.

mod access;
mod attention;
mod attention_convert;
mod attention_grpc;
mod convert;
mod grpc;
mod plugin;
mod service;
mod store;

pub(crate) use grpc::server;
pub(crate) use plugin::Dashboards;
