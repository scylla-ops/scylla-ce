//! The library of a workspace: the `fragments` tables and `FragmentService`. The fragments that
//! the pipelines link are in `pipelines.fragment_links`. An edition reads the versions of a
//! fragment with `versions`.

mod access;
mod convert;
mod grpc;
mod origin_grpc;
mod origin_service;
mod plugin;
mod service;
mod store;
#[cfg(test)]
mod tests;

pub(crate) use plugin::Library;
pub use service::versions;
