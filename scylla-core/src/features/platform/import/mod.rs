//! The one-off import of the database of the old product: the users with their password hashes
//! and their personal workspaces, the organizations as workspaces with their owners and roles,
//! the projects with their roles, their secrets sealed again, and their pipelines.

mod content;
mod ids;
mod legacy;
mod nodes;
mod service;
mod tenancy;
mod users;

pub use service::{import, Report};
