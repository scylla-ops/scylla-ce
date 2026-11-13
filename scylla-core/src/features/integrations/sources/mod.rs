//! The commit context of a run, a run on a branch, a tag or a SHA of the linked repository, and
//! the files that a pull request changes.

mod changes;
mod commits;
pub(crate) mod convert;
mod github;
mod gitlab;
mod mount;
mod store;

pub(crate) use changes::changes_pipeline_files;
pub(crate) use commits::resolve;
pub(crate) use mount::mount;
