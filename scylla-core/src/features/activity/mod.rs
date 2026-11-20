//! The activity feed of a workspace: the rows that the services record in the transaction of each
//! change, `ActivityService`, and the delete of the old rows.

mod access;
mod convert;
mod grpc;
mod plugin;
mod record;
mod service;
mod store;

#[cfg(test)]
mod tests;

pub(crate) use plugin::Activities;
pub(crate) use record::{
    agent, connection, fire_failed, job_ended, joined, record, role, secret, time_missed, trigger,
};
pub(crate) use service::purge;
