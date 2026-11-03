//! The accounts feature: users with a profile, a handle and an email, passwords, the one-time
//! links of the mails, the deletion of a user, `AccountService` and `ProfileService`.

mod context;
mod convert;
mod deletion;
mod grpc;
mod letter;
pub(crate) mod mails;
mod manage;
mod one_time;
mod password;
mod profile;
mod reset;
mod service;
mod store;

pub use context::{AccountContext, AccountSettings};
pub(crate) use convert::{profile_change, session};
pub(crate) use deletion::delete_user;
pub(crate) use grpc::{deliver, server as account_server, to_proto};
pub(crate) use manage::{change_profile, get_user, list_users, revoke_sessions};
pub(crate) use profile::server as profile_server;
pub(crate) use reset::send_reset;
pub use service::{create_admin, create_user, PERSONAL};
pub(crate) use service::{created, insert_user, refused, signed_in};
pub(crate) use store::{
    find_user, lock_user, record_sign_in, set_status, user_of_email, verified_emails,
};
