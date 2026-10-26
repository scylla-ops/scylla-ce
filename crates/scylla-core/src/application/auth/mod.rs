pub mod hash_service;
pub mod session_repository;

pub use hash_service::HashService;
pub use session_repository::SessionRepository;

use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::UserId;
use crate::domain::session::Session;
use chrono::Duration;
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

/// The one session rule, for the interceptor and for `validate_token`. It only reads: the
/// `purge_expired_sessions` pass deletes an expired session.
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
