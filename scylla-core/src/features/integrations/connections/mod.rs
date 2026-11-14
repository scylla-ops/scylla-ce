//! The GitHub and GitLab connections of a workspace: `VcsConnectionService`, the install of the
//! GitHub App, the token of a GitLab service account, and the credentials that the other parts
//! of the integration use.

pub(crate) mod calls;
pub(crate) mod checks;
mod convert;
pub(crate) mod credentials;
mod grpc;
mod install;
mod mount;
mod redirects;
mod repositories;
mod service;
mod setup_grpc;
pub(crate) mod store;
mod tokens;

#[cfg(test)]
mod tests;

pub(crate) use mount::mount;
