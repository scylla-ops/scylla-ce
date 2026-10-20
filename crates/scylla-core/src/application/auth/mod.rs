pub mod commands;
pub mod hash_service;
pub mod queries;
pub mod session_repository;
pub mod sweeper;

pub use commands::{Login, PurgeExpiredSessions, RevokeToken};
pub use hash_service::HashService;
pub use queries::ValidateToken;
pub use session_repository::SessionRepository;
pub use sweeper::SessionSweeper;

use crate::application::UserRepository;
use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::UserId;
use crate::domain::session::Session;
use chrono::Duration;
use derive_more::Constructor;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

const SESSION_TTL_HOURS: i64 = 24;

/// A random opaque token, valid for one day. Every sign-in path stages one.
#[must_use]
pub fn new_session(user_id: UserId) -> Session {
    Session::create(
        user_id,
        Uuid::new_v4().to_string(),
        Duration::hours(SESSION_TTL_HOURS),
    )
}

pub enum SessionLookup {
    Live(Session),
    Expired,
    Unknown,
}

/// The one session rule, for the interceptor and for `ValidateToken`. It only reads: the
/// `PurgeExpiredSessions` pass deletes an expired session.
pub async fn look_up_session(
    sessions: &dyn SessionRepository,
    token: &str,
) -> DomainResult<SessionLookup> {
    match sessions.find_by_token(token).await {
        Ok(session) if session.is_expired() => Ok(SessionLookup::Expired),
        Ok(session) => Ok(SessionLookup::Live(session)),
        Err(e) if e.is_not_found() => Ok(SessionLookup::Unknown),
        Err(e) => Err(e),
    }
}

/// Moves the last activity of a live session, at most once every `session::ACTIVITY_INTERVAL`,
/// so most calls write nothing. A failure is logged and does not refuse the call.
pub async fn record_activity(sessions: &dyn SessionRepository, session: &Session) {
    let now = clock::now();
    if !session.activity_due(now) {
        return;
    }
    if let Err(error) = sessions.touch(session.id(), now).await {
        warn!(session_id = %session.id(), %error, "cannot record the session activity");
    }
}

/// The session's stage runners, one block per action in `commands.rs` and `queries.rs`. Every
/// action is `Public`: the caller holds no session yet, or holds the token it sends.
#[derive(Constructor)]
pub struct AuthUseCases {
    pub(super) user_repo: Arc<dyn UserRepository>,
    pub(super) session_repo: Arc<dyn SessionRepository>,
    pub(super) hash_service: Arc<dyn HashService>,
}

#[cfg(test)]
mod tests;
