use crate::application::grant::PrincipalAccess;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{SessionId, UserId};
use crate::domain::session::SessionClient;
use crate::domain::user::{PasswordReset, ResetToken, User};
use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};

/// One session of a user, without its token. `current` marks the session of the call; the store
/// leaves it `false`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserSession {
    pub id: SessionId,
    pub created_at: DateTime<Utc>,
    pub last_active_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub client: SessionClient,
    pub current: bool,
}

/// What a user signs in with, beyond its row: its sessions, its reset links, and its grants with
/// their names. A write that also writes the user follows the version rule of
/// `UserRepository::update`, and does all its work in one transaction.
#[async_trait]
pub trait AccountRepository: Send + Sync {
    /// Writes the user, deletes each session of the user except `keep`, and deletes each reset
    /// link of the user.
    async fn update_signed_out(&self, user: &User, keep: Option<&SessionId>) -> DomainResult<User>;

    /// Deletes each session of the user except `keep`, and returns the count.
    async fn revoke_sessions(
        &self,
        user_id: &UserId,
        keep: Option<&SessionId>,
    ) -> DomainResult<u64>;

    /// The sessions of the user that have not expired, the most recently active first.
    async fn list_sessions(&self, user_id: &UserId) -> DomainResult<Vec<UserSession>>;

    /// Deletes the session when it is a session of the user that has not expired. `false` when
    /// it deleted nothing.
    async fn revoke_session(&self, user_id: &UserId, id: &SessionId) -> DomainResult<bool>;

    /// Stores the link and deletes the other links of the user. With a `cooldown`, it stores
    /// nothing and returns `false` when the user has a link made within it.
    async fn issue_reset(
        &self,
        reset: &PasswordReset,
        cooldown: Option<Duration>,
    ) -> DomainResult<bool>;

    /// `NotFound` for a token that no link has.
    async fn find_reset(&self, token: &ResetToken) -> DomainResult<PasswordReset>;

    /// Marks the link used, writes the user, deletes the other links and each session of the
    /// user. A link that is used or expired at the write gives `reset_link_invalid`.
    async fn redeem_reset(&self, reset: &PasswordReset, user: &User) -> DomainResult<User>;

    /// The system grants first, then by organization name, then by project name.
    async fn list_access(&self, user_id: &UserId) -> DomainResult<Vec<PrincipalAccess>>;
}
