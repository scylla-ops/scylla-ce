use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::SessionId;
use crate::domain::session::Session;
use async_trait::async_trait;
use chrono::{DateTime, Utc};

#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn create(&self, session: &Session) -> DomainResult<Session>;

    async fn find_by_token(&self, token: &str) -> DomainResult<Session>;

    async fn delete_by_token(&self, token: &str) -> DomainResult<()>;

    async fn delete_expired(&self) -> DomainResult<u64>;

    /// Sets the last activity of the session to `at` in one conditional write, only when the
    /// stored value is `session::ACTIVITY_INTERVAL` old or older at `at`. `false` when it wrote
    /// nothing. The default refuses, so a store that existed before this method still compiles;
    /// the Postgres store writes.
    async fn touch(&self, id: &SessionId, at: DateTime<Utc>) -> DomainResult<bool> {
        let _ = (id, at);
        Err(DomainError::internal(
            "this session store does not record the session activity",
        ))
    }
}
