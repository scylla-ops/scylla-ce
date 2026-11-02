//! The test kit: the real server of an edition, with a real gate, a mailbox, and helpers for the
//! core RPCs, grouped by subject.

mod app;
mod identity;
mod mail;
mod platform;
mod projects;
mod rpc;
mod tenancy;

pub use app::{test_access, TestApp};
pub use mail::Mailbox;
pub use rpc::{id, text, with_token};
pub use tenancy::Role;
