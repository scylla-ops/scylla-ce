//! The rules that block a merge when a pipeline fails: `MergeBlockService`, the ruleset of
//! Scylla on GitHub with one required check for each pipeline, and "Pipelines must succeed" on
//! GitLab.

mod apply;
mod calls;
mod convert;
mod grpc;
mod mount;
mod rules;
mod service;
mod store;
#[cfg(test)]
mod tests;
mod view;

pub(crate) use mount::mount;
