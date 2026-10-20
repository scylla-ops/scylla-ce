use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::organization::Organization;
use crate::domain::user::User;
use async_trait::async_trait;
use scylla_auth::authz::Grant;

#[async_trait]
pub trait SignupRepository: Send + Sync {
    async fn provision_account(
        &self,
        user: &User,
        organization: &Organization,
        grants: &[Grant],
    ) -> DomainResult<()>;

    /// Writes an account without an organization: the user and its grants, in one transaction.
    /// The default refuses, so a store that existed before this method still compiles; the
    /// Postgres store writes the account.
    async fn provision_user(&self, user: &User, grants: &[Grant]) -> DomainResult<()> {
        let _ = (user, grants);
        Err(DomainError::internal(
            "this signup store cannot write an account without an organization",
        ))
    }
}
