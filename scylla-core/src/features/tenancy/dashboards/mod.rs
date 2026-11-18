//! The saved layouts of the dashboard of a workspace: the `dashboards` table and
//! `DashboardService`.

mod access;
mod convert;
mod grpc;
mod plugin;
mod service;
mod store;

pub(crate) use plugin::Dashboards;
