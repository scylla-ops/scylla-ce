//! The users feature: persons who sign in with an OIDC identity provider.

mod oidc;
mod service;
mod store;

pub use oidc::OidcVerifier;
pub(crate) use service::sign_in;
