//! The notifications: the facts that the services record, the task that turns them into rows, and
//! `NotificationService`, the inbox of a user.

mod audience;
mod dispatcher;
mod emit;
mod grpc;
pub(crate) mod kinds;
mod plugin;
pub mod purge;
mod service;
mod store;

pub use dispatcher::dispatch_pending;
pub(crate) use dispatcher::spawn;
pub(crate) use emit::{close, emit, reopen, withdraw, PgNotifier};
pub(crate) use plugin::Notifications;
