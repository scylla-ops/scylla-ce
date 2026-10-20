pub mod commands;
pub mod credential_repository;
pub mod queries;
pub mod repository;
pub mod secret_mint;
pub mod token;

pub use commands::{
    CreateApp, CreateAppSecret, CreatedApp, CreatedAppSecret, DeleteApp, NewApp, NewAppSecret,
    RevokeAppSecret, SetAppActive, SetAppSecretEnabled, SetAppSecretLabel,
};
pub use credential_repository::AppCredentialRepository;
pub use queries::{GetApp, ListAppAccess, ListAppSecrets, ListApps};
pub use repository::AppRepository;
pub use secret_mint::mint_app_secret;
pub use token::{AppTokenRepository, AppTokenUseCases, IssueAppToken};

use crate::application::HashService;
use crate::application::agent::dispatch_port::AgentDispatch;
use derive_more::Constructor;
use std::sync::Arc;

/// The app aggregate's stage runners, one block per action in `commands.rs` and `queries.rs`.
/// It has no method of its own; `Actions::run` drives it.
#[derive(Constructor)]
pub struct AppUseCases {
    pub(super) app_repo: Arc<dyn AppRepository>,
    pub(super) credential_repo: Arc<dyn AppCredentialRepository>,
    pub(super) hash_service: Arc<dyn HashService>,
    pub(super) registry: Arc<dyn AgentDispatch>,
}

#[cfg(test)]
mod tests;
