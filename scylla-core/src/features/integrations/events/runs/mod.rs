//! The runs of the Git events: the editor pipelines of a linked project whose enabled Git
//! triggers match a push, a pull request or a tag get one run for each commit.

mod start;
mod store;

pub(crate) use start::handle;
