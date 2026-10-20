use crate::domain::app::AppToken;
use crate::domain::errors::DomainResult;
use async_trait::async_trait;

#[async_trait]
pub trait AppTokenRepository: Send + Sync {
    async fn create(&self, token: &AppToken) -> DomainResult<()>;
    async fn find_by_token(&self, token: &str) -> DomainResult<AppToken>;
}
