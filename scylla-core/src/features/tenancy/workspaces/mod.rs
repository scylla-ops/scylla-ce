//! The workspaces feature: the `workspaces` table and `WorkspaceService`.

mod grpc;
mod plugin;
mod service;
mod store;

pub(crate) use plugin::Workspaces;
pub use service::create;
pub(crate) use store::{delete_all, has_content, insert, leave_all, lock_sole_owned};
