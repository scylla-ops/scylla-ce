//! The roles feature: `RoleService`, the catalog of the permissions, the custom roles, and the
//! roles of the platform level that `AccessService` gives.

mod convert;
mod grpc;
mod platform;
mod plugin;
mod service;
mod store;

pub(crate) use platform::{give_on_platform, make_workspace_creator, take_back_on_platform};
pub(crate) use plugin::Roles;
pub(crate) use store::delete_grants_of;
