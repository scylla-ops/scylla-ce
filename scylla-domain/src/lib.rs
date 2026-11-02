//! The pure types of the core features: ids, actors, the permission model, validated values,
//! entities and pages.

extern crate self as scylla_domain;

pub mod actors;
pub mod agents;
pub mod authz;
mod codec;
mod error;
mod id;
pub mod page;
pub mod projects;
mod resource;
pub mod users;

pub use error::DomainError;
pub use id::Id;
pub use resource::ResourceKind;
pub use scylla_macros::authz;
