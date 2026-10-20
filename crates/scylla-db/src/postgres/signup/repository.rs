use crate::domain::errors::DomainResult;
use crate::domain::organization::Organization;
use crate::domain::user::User;
use async_trait::async_trait;
use scylla_auth::authz::Grant;
use scylla_core::application::signup::repository::SignupRepository;
use sqlx::{PgPool, Postgres, Transaction};
use tracing::instrument;

use super::super::error::SqlxResultExt;
use super::super::{grants, organizations, users};

#[derive(Clone)]
pub struct PgSignupRepository {
    pool: PgPool,
}

impl PgSignupRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl SignupRepository for PgSignupRepository {
    #[instrument(skip_all, fields(user_id = %user.id(), org_id = %organization.id()))]
    async fn provision_account(
        &self,
        user: &User,
        organization: &Organization,
        account_grants: &[Grant],
    ) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.to_domain()?;

        users::repository::queries::create(&mut *tx, user).await?;
        organizations::repository::queries::create(&mut *tx, organization).await?;
        for grant in account_grants {
            grants::insert(&mut *tx, grant).await?;
        }

        tx.commit().await.to_domain()?;
        Ok(())
    }

    #[instrument(skip_all, fields(user_id = %user.id()))]
    async fn provision_user(&self, user: &User, account_grants: &[Grant]) -> DomainResult<()> {
        let mut tx = self.pool.begin().await.to_domain()?;
        provision_user_in(&mut tx, user, account_grants).await?;
        tx.commit().await.to_domain()?;
        Ok(())
    }
}

/// `provision_user` in a transaction that the caller owns: an edition writes its own rows in the
/// same transaction, then commits. A taken username or email is `Conflict`.
pub async fn provision_user_in(
    tx: &mut Transaction<'_, Postgres>,
    user: &User,
    account_grants: &[Grant],
) -> DomainResult<()> {
    users::repository::queries::create(&mut **tx, user).await?;
    for grant in account_grants {
        grants::insert(&mut **tx, grant).await?;
    }
    Ok(())
}
