//! The providers feature: the identity providers of the sign-in (OIDC and GitHub), the
//! identities of the users, and the browser routes of a sign-in.

mod calls;
mod failure;
mod github;
mod http;
mod identity;
mod join;
mod oidc;
mod service;
mod store;

pub use github::GitHubSettings;
pub use http::router;
pub(crate) use identity::ExternalIdentity;
pub use oidc::OidcSettings;
pub(crate) use service::list_enabled;
pub use service::Providers;
pub use store::{by_slug, list, upsert, IdentityInfo, ProviderRow};
pub(crate) use store::{identities_of, unlink};
