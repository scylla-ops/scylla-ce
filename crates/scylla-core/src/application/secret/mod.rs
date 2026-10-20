pub mod cipher;
pub mod commands;
pub mod queries;
pub mod repository;
pub mod resolver;

pub use cipher::SecretCipher;
pub use commands::{CreateSecret, DeleteSecret};
pub use queries::ListSecrets;
pub use repository::SecretRepository;
pub use resolver::{DispatchSecretResolver, SecretResolver};

use derive_more::Constructor;
use std::sync::Arc;

/// The secret aggregate's stage runners, one block per action in `commands.rs` and
/// `queries.rs`. It has no method of its own; `Actions::run` drives it.
#[derive(Constructor)]
pub struct SecretUseCases {
    pub(super) secret_repo: Arc<dyn SecretRepository>,
    pub(super) cipher: Arc<dyn SecretCipher>,
}

#[cfg(test)]
mod tests;
