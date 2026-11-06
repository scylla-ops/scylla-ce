//! The pure types of the core features: the kernel (ids, names, pages, errors), the permission
//! model, and the entities of each subject.

extern crate self as scylla_domain;

pub mod authz;
pub mod identity;
mod kernel;
pub mod notifications;
pub mod platform;
pub mod projects;
pub mod tenancy;

pub use kernel::{DomainError, Id, Name, Page, PageRequest, PageSize, ResourceKind};
pub use scylla_macros::authz;
