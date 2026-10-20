use crate::domain::app::{AppSecret, AppSecretHash};
use crate::domain::errors::DomainResult;
use crate::domain::user::{Password, PasswordHash};
use async_trait::async_trait;

#[async_trait]
pub trait HashService: Send + Sync {
    async fn hash(&self, password: &Password) -> DomainResult<PasswordHash>;

    async fn verify(&self, password: &Password, hash: &PasswordHash) -> DomainResult<bool>;

    async fn hash_secret(&self, secret: &AppSecret) -> DomainResult<AppSecretHash>;

    async fn verify_secret(&self, secret: &AppSecret, hash: &AppSecretHash) -> DomainResult<bool>;
}
