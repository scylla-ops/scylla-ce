//! The triggers feature: the `triggers`, `trigger_runners`, `trigger_deliveries` and
//! `trigger_fires` tables, `TriggerService`, the public webhook route, and the loop of the cron
//! schedules.

mod convert;
pub(crate) mod fire;
mod grpc;
mod history;
pub(crate) mod inputs;
mod lists_convert;
mod lists_grpc;
mod lists_proto;
mod plugin;
pub(crate) mod schedule;
mod service;
mod store;
mod summaries;
pub(crate) mod webhook;

#[cfg(test)]
mod tests;

pub(crate) use grpc::server;
pub(crate) use plugin::Triggers;
pub(crate) use service::git_rules;
