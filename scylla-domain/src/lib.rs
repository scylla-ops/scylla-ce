//! The pure types of the core features: the kernel (ids, names, pages, errors), the permission
//! model, the entities of each subject, and the activity feed.

extern crate self as scylla_domain;

pub mod activity;
pub mod authz;
pub mod identity;
pub mod integrations;
mod kernel;
pub mod notifications;
pub mod platform;
pub mod projects;
pub mod tenancy;

pub use kernel::{
    DomainError, Id, Name, Page, PageRequest, PageSize, ResourceKind, Span, TimeZone,
};
pub use scylla_macros::authz;
