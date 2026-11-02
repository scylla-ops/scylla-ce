//! The accounts feature: users with a profile and an email, passwords, the one-time links of the
//! mails, `AccountService` and `ProfileService`.

mod context;
mod grpc;
pub(crate) mod mails;
mod one_time;
mod password;
mod profile;
mod service;
mod store;

pub use context::{AccountContext, AccountSettings};
pub(crate) use grpc::{deliver, server as account_server, to_proto, user_agent};
pub(crate) use profile::server as profile_server;
pub use service::{create_user, PERSONAL};
pub(crate) use service::{insert_user, refused};
pub(crate) use store::{find_user, record_sign_in, user_of_email, verified_emails};
