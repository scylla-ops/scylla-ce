//! The commit context of a run, and a run on a branch, a tag or a SHA of the linked repository.

mod commits;
pub(crate) mod convert;
mod github;
mod gitlab;
mod mount;
mod store;

pub(crate) use commits::resolve;
pub(crate) use mount::mount;
