//! The checkout of a job on an agent, with a short-lived token: an installation token of the
//! GitHub App, or a GitLab deploy token that Scylla deletes when no job needs it.

mod cleanup;
mod dispatch;
mod github;
mod gitlab;
mod mount;
mod store;
#[cfg(test)]
mod tests;

pub(crate) use dispatch::{prepare, Checkout};
pub(crate) use mount::mount;
