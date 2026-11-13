//! The triggers feature: the `triggers`, `trigger_runners` and `trigger_deliveries` tables,
//! `TriggerService`, the public webhook route, and the loop of the cron schedules.

mod convert;
pub(crate) mod fire;
mod grpc;
pub(crate) mod inputs;
mod plugin;
pub(crate) mod schedule;
mod service;
mod store;
pub(crate) mod webhook;

#[cfg(test)]
mod tests;

pub(crate) use grpc::server;
pub(crate) use plugin::Triggers;
pub(crate) use service::git_rules;
