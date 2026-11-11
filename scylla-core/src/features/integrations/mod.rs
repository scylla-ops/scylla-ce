//! The GitHub and GitLab integration: the shared context, and one folder for each part (the
//! instance Apps, the connections, the repository links, the commit context of the runs, the
//! provider events, the polling, the reports, the checkout, the merge blocks and the pipeline
//! files). `mount` adds their services, public routes and tasks to the server.

pub(crate) mod apps;
pub(crate) mod checkout;
pub(crate) mod connections;
mod context;
pub(crate) mod events;
pub(crate) mod files;
pub(crate) mod links;
pub(crate) mod merge_blocks;
mod mount;
pub(crate) mod polling;
pub(crate) mod reports;
pub(crate) mod sources;

pub(crate) use context::IntegrationContext;
pub(crate) use mount::{mount, Part};
