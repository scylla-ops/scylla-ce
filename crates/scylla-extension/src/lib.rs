//! The action pipeline: every write is a command that moves through typed phases, and an
//! extension attaches to each stage. Depends on the kernel only, so an out-of-tree edition builds
//! against a pinned tag without the access model, the database or the gRPC stack.

/// `no_inline`: rustdoc would otherwise copy the whole model into this crate's docs.
#[doc(no_inline)]
pub use scylla_domain::domain;

pub mod action;
pub mod actions;
pub mod authz;
pub mod hooks;
pub mod path;
pub mod stage;

pub use action::{
    Action, ActionId, Authorized, Command, Committed, Deleted, Describe, Draft, Envelope, Fetched,
    Phase, Prepared, Query, Requested,
};
pub use actions::Actions;
pub use authz::{Access, AuthorizeStage, Authorizer, Granted};
pub use hooks::{
    Around, Done, Extension, Gate, Hooks, Listener, Next, Observer, Policy, Proceed, Wrap,
};
pub use path::{Kind, Path, Read, Write};
pub use stage::{Authorize, Fetch, Persist, Prepare, Run, Stage, StageKind};

#[cfg(test)]
mod tests;
