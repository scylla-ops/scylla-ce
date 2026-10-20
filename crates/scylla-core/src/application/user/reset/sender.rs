use crate::domain::errors::DomainResult;
use crate::domain::ids::UserId;
use crate::domain::user::{DisplayName, Email, Username};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::fmt;

/// How a server delivers its reset links. It depends on the sender only, never on the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordResetDelivery {
    Mail,
    ServerLog,
}

/// A reset link to deliver to one user. `link` carries the token: a sender writes it only to its
/// recipient.
#[derive(Clone)]
pub struct PasswordResetMessage {
    pub user_id: UserId,
    pub email: Email,
    pub username: Username,
    pub display_name: Option<DisplayName>,
    pub link: String,
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for PasswordResetMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PasswordResetMessage")
            .field("user_id", &self.user_id)
            .field("username", &self.username)
            .field("link", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// The port that delivers reset links. The server has one sender: the default writes the link in
/// the server log, and an edition replaces it with `Server::password_reset_sender`. The server
/// can log an error of `send`, so the error must not hold the email or the link.
#[async_trait]
pub trait PasswordResetSender: Send + Sync {
    fn delivery(&self) -> PasswordResetDelivery;

    async fn send(&self, message: &PasswordResetMessage) -> DomainResult<()>;
}
