//! The mail settings feature: the SMTP relay of the instance, `MailSettingsService`, for its
//! admins. The relay replaces the default mailer of the server at the next send, with no restart.

mod convert;
mod grpc;
mod service;
mod store;

pub(crate) use grpc::server;
pub(crate) use service::load;
