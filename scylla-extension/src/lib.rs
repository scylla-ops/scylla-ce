//! The contract between the core and the editions: plugins, hooks and the access gate.

mod context;
mod edition;
mod error;
mod gate;
mod hooks;
mod tx;

pub use async_trait::async_trait;
pub use context::{CreateContext, DeleteContext};
pub use edition::{Edition, EditionBuilder, Plugin, ServiceContext};
pub use error::{HookError, Rejection};
pub use gate::{AccessError, Gate, IssuedToken, Operation, Principal};
pub use hooks::{AfterDelete, BeforeCreate, Hooks};
pub use tx::Tx;
