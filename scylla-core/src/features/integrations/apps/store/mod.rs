//! The SQL of the GitHub App of the instance: one file for its row in `vcs_apps`, one for the
//! states of the manifest flow, one for its last webhook, and one for its connections.

mod apps;
pub(crate) mod connections;
pub(crate) mod deliveries;
pub(crate) mod states;

pub(crate) use apps::{delete, find, github, insert, lock, NewApp, SealedApp};
