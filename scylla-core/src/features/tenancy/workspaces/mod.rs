//! The workspaces feature: the `workspaces` and `workspace_owners` tables, and
//! `WorkspaceService`.

mod convert;
mod grpc;
mod plugin;
mod service;
mod store;

pub(crate) use plugin::Workspaces;
pub use service::create;
pub(crate) use service::{give_owner, refusing_personal_tokens, take_owner};
pub(crate) use store::{add_owner, delete_all, has_content, insert, leave_all, lock_sole_owned};
