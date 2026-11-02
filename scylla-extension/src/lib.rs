//! The contract between the core and the editions: plugins, hooks and authenticators.

mod auth;
mod context;
mod edition;
mod error;
mod hooks;
mod tx;

pub use async_trait::async_trait;
pub use auth::{AuthError, Authenticator, Authenticators};
pub use context::{CreateContext, DeleteContext};
pub use edition::{Edition, EditionBuilder, Plugin, ServiceContext};
pub use error::{HookError, Rejection};
pub use hooks::{AfterDelete, BeforeCreate, Hooks};
pub use tx::Tx;
