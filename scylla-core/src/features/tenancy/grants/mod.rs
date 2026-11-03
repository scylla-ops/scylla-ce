//! The grants feature: `AccessService`, that gives, takes back and lists the roles that the
//! plugins register with `grants` and `role`, and that tells a caller its permissions.

mod check;
mod grpc;
mod list;
mod plugin;
mod service;
mod store;

pub(crate) use plugin::Grants;
