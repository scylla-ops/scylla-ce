//! The grants feature: `AccessService`, that gives and takes back the roles that the plugins
//! register with `grants` and `role`.

mod grpc;
mod plugin;

pub(crate) use plugin::Grants;
