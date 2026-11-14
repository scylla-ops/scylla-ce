//! The runs of the Git events: the editor pipelines of a linked project whose enabled Git
//! triggers match a push, a pull request or a tag get one run for each commit, and in a project
//! of files, the file pipelines whose file at the commit matches the event.

mod file_pipelines;
mod start;
mod store;

pub(crate) use start::handle;
