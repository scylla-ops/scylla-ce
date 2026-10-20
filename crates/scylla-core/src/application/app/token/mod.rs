pub mod commands;
pub mod repository;

pub use commands::IssueAppToken;
pub use repository::AppTokenRepository;

use crate::application::HashService;
use crate::application::app::{AppCredentialRepository, AppRepository};
use derive_more::Constructor;
use std::sync::Arc;

/// The app token exchange's stage runners, one block per action in `commands.rs`.
/// `IssueAppToken` is `Public`: the app secret is the credential.
#[derive(Constructor)]
pub struct AppTokenUseCases {
    pub(super) app_repo: Arc<dyn AppRepository>,
    pub(super) token_repo: Arc<dyn AppTokenRepository>,
    pub(super) credential_repo: Arc<dyn AppCredentialRepository>,
    pub(super) hash_service: Arc<dyn HashService>,
}

#[cfg(test)]
mod tests;
