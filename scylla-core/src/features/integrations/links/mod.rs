//! The repository link of a project: `RepositoryService`, the check that a connection sees the
//! repository, the webhook of a GitLab project, and the branches and the tags of the repository.

mod calls;
mod convert;
mod grpc;
mod link;
mod mount;
mod refs;
mod service;
pub(crate) mod store;
mod test_delivery;

pub(crate) use mount::mount;
