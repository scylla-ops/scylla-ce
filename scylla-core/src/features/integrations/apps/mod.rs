//! The GitHub App of an instance, made with the manifest flow: `VcsAppService`, the manifest and
//! its state, the conversion of the code, and the read of the App by the other parts.

mod calls;
mod convert;
mod grpc;
mod manifest;
mod mount;
mod service;
pub(crate) mod store;

pub(crate) use mount::mount;
