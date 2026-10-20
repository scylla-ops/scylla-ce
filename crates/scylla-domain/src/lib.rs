//! No sqlx, tonic, cedar or argon2 here: the agent links this crate.

pub mod domain;

pub use domain::job::JobEvent;
