//! The pipelines from the files of a repository: the read of the files at a commit, the sync of
//! the file pipelines with the default branch, the runs of the files at a commit, the source of
//! the pipelines of a project, and the JSON Schema of the files.

mod cache;
mod github;
mod gitlab;
mod mount;
mod read;
mod repo;
mod runs;
mod schema;
mod source;
mod store;
mod sync;
#[cfg(test)]
mod tests;

pub(crate) use mount::mount;
pub(crate) use read::read_at;
pub(crate) use runs::{head_source, start_at, starts_of_event};
pub(crate) use source::set_source;
pub(crate) use sync::{refresh, sync_default_branch};
