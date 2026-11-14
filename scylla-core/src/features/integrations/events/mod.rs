//! The webhooks of the providers, their Git events, and the dispatch of the events: the runs, the
//! re-runs, and the changes of the installations of the GitHub App.

mod answers;
mod dispatch;
mod github;
mod gitlab;
mod installations;
mod mount;
mod prune;
mod receive;
mod rerun;
mod runs;
mod store;

#[cfg(test)]
mod tests;

pub(crate) use dispatch::{polled, Scope};
pub(crate) use mount::mount;
pub(crate) use store::Delivery;
