//! The contract between the core and the editions: plugins, hooks and the access gate.

mod context;
mod edition;
mod error;
mod gate;
mod grants;
mod hooks;
mod mail;
mod operation;
#[cfg(test)]
mod tests;
mod tx;

pub use async_trait::async_trait;
pub use context::{CreateContext, DeleteContext};
pub use edition::{Edition, EditionBuilder, Plugin, ServiceContext};
pub use error::{HookError, Rejection};
pub use gate::{AccessError, Binding, Gate, IssuedToken, Principal, RoleHolder, RolePage};
pub use grants::{AccessRoutes, FindWorkspace, Grantable, Grants, WorkspaceFuture};
pub use hooks::{AfterDelete, BeforeCreate, Hooks};
pub use mail::{Mail, MailError, Mailer};
pub use operation::Operation;
pub use tx::Tx;
