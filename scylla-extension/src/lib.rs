//! The contract between the core and the editions: plugins, hooks and the access gate.

mod context;
mod edition;
mod error;
mod gate;
mod grants;
mod hooks;
mod mail;
mod notification_events;
mod notifications;
mod operation;
mod principal;
mod service_context;
#[cfg(test)]
mod tests;
mod tx;

pub use async_trait::async_trait;
pub use context::{CreateContext, DeleteContext};
pub use edition::{Edition, EditionBuilder, Plugin, WebFiles};
pub use error::{HookError, Rejection};
pub use gate::{AccessError, Gate, ObjectRole, RoleHolder, RolePage};
pub use grants::{AccessRoutes, FindWorkspace, Grantable, Grants, WorkspaceFuture};
pub use hooks::{AfterDelete, BeforeCreate, Hooks};
pub use mail::{Mail, MailError, Mailer};
pub use notification_events::{
    Audience, AudienceContext, Event, EventRow, NotificationError, Notifier, Params, Recipient,
    REASON_SUBJECT,
};
pub use notifications::{Access, Collapse, NotificationKind, NotificationKinds};
pub use operation::Operation;
pub use principal::{Binding, IssuedToken, Principal};
pub use service_context::ServiceContext;
pub use tx::Tx;
