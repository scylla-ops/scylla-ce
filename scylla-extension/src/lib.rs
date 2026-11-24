//! The contract between the core and the editions: plugins, hooks, background tasks and the
//! access gate.

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
mod tasks;
#[cfg(test)]
mod tests;
mod tx;

pub use async_trait::async_trait;
pub use context::{CreateContext, DeleteContext, ExportPart, UserContext};
pub use edition::{Edition, EditionBuilder, Plugin, WebFiles};
pub use error::{HookError, Rejection};
pub use gate::{AccessError, Gate, ObjectRole, RoleHolder, RolePage};
pub use grants::{AccessRoutes, FindWorkspace, Grantable, Grants, WorkspaceFuture};
pub use hooks::{AfterDelete, AfterDeleteUser, BeforeCreate, BeforeDelete, ExportUser, Hooks};
pub use mail::{Mail, MailError, MailImage, Mailer};
pub use notification_events::{
    Audience, AudienceContext, Event, EventRow, NotificationError, Notifier, Params, Recipient,
    REASON_SUBJECT,
};
pub use notifications::{Access, Collapse, NotificationKind, NotificationKinds};
pub use operation::Operation;
pub use principal::{Binding, IssuedToken, Principal};
pub use service_context::ServiceContext;
pub use tasks::{ScheduledTask, Task, TaskContext, TaskError};
pub use tx::Tx;
