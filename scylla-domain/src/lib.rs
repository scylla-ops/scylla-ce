//! The pure types of the core features: ids, actors, validated values, entities and pages.

pub mod actors;
pub mod agents;
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
