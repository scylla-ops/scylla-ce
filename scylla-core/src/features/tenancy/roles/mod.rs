//! The roles feature: `RoleService`, the catalog of the permissions, the custom roles, and the
//! roles of the platform level that `AccessService` gives.

mod changes;
mod convert;
mod grpc;
mod holders;
mod moving;
mod platform;
mod plugin;
pub(crate) mod service;
pub(crate) mod store;

pub(crate) use platform::{give_on_platform, take_back_on_platform};
pub(crate) use plugin::Roles;
pub(crate) use store::delete_grants_of;
