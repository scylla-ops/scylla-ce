//! The permission model: typed resources, relations and permissions, checks, and the schema.

mod check;
mod schema;
#[cfg(test)]
mod tests;
mod typed;

pub use check::{Check, Object, Relationship};
pub use schema::{Model, Part, Schema, SchemaError};
pub use typed::{Permission, Relation, Resource};
