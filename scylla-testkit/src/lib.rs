//! The test kit: the real server of an edition, with a real gate, a mailbox, a capture of the
//! log lines, and helpers for the core RPCs, grouped by subject.

mod app;
mod identity;
mod logs;
mod mail;
mod pipelines;
mod platform;
mod projects;
mod rpc;
mod tenancy;
mod triggers;

pub use app::{test_access, TestApp};
pub use identity::{new_key, TestKey};
pub use logs::Logs;
pub use mail::Mailbox;
pub use pipelines::exec_node;
pub use rpc::{id, text, with_token};
pub use tenancy::{Role, RoleInput};
pub use triggers::{literal_input, pointer_input};
