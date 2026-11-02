//! The test kit: the real server of an edition, with a real gate, a mailbox, and helpers for the
//! core RPCs.

mod accounts;
mod app;
mod mail;
mod rpc;
mod workspaces;

pub use app::{test_access, TestApp};
pub use mail::Mailbox;
pub use rpc::{id, text, with_token, Role};
