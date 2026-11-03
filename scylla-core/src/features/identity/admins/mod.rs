//! The admins feature: the instance admins in `instance_admins`, the rule of the last active
//! admin, and `UserAdminService`, the console of the users.

mod grpc;
mod service;
mod store;

pub(crate) use grpc::server;
pub use service::{grant, revoke};
pub(crate) use service::{grant_in, keep_an_admin};
