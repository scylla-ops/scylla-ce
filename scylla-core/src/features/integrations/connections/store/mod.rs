//! The SQL of the connections: one file for the reads of `vcs_connections`, one for its writes,
//! one for the health of the connections, one for the changes that the GitHub webhooks tell, and
//! one for the states of the installs.

mod connections;
pub(crate) mod health;
pub(crate) mod installations;
mod row;
pub(crate) mod states;
mod writes;

pub(crate) use connections::{delete, find, hooks, links, list, project_workspace, sealed_token};
pub(crate) use row::Counted;
pub(crate) use writes::{insert_token, replace_token, upsert_installation, Installation, Token};
