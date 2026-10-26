pub mod credential_repository;
pub mod repository;
pub mod secret_mint;
pub mod token;

pub use credential_repository::AppCredentialRepository;
pub use repository::AppRepository;
pub use secret_mint::mint_app_secret;
pub use token::AppTokenRepository;

use crate::domain::app::{App, AppCredential, AppSecret};

/// The outcome of `create_app`. No `Debug`: `secret` is the plaintext the caller sees once.
pub struct CreatedApp {
    pub app: App,
    pub secret: AppSecret,
}

/// The outcome of `create_app_secret`. No `Debug`: `secret` is the plaintext the caller sees once.
pub struct CreatedAppSecret {
    pub credential: AppCredential,
    pub secret: AppSecret,
}
