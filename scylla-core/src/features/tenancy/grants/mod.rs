//! The grants feature: `AccessService`, that gives, takes back and lists the roles that the
//! plugins register with `grants` and `role`, and that tells a caller its permissions.

mod check;
mod effective;
mod give;
mod grpc;
pub(crate) mod held;
mod held_values;
mod list;
mod members;
pub(crate) mod membership;
mod owner;
mod platform;
mod plugin;
pub(crate) mod service;
pub(crate) mod store;

pub(crate) use plugin::Grants;
