//! The pipelines from the files of a repository: the read of the files at a commit, the sync of
//! the file pipelines with the default branch, the source of the pipelines of a project, and the
//! JSON Schema of the files.

mod cache;
mod github;
mod gitlab;
mod mount;
mod read;
mod repo;
mod schema;
mod source;
mod store;
mod sync;
#[cfg(test)]
mod tests;

pub(crate) use mount::mount;
pub(crate) use read::read_at;
pub(crate) use source::set_source;
pub(crate) use sync::refresh;
#[allow(unused_imports, reason = "the runs of the file pipelines call it")]
pub(crate) use sync::sync_default_branch;
