use async_trait::async_trait;
use sqlx::{Postgres, Transaction};

use crate::{CreateContext, DeleteContext, GuardError};

/// Runs inside the caller's transaction. May reject, and may write to its own tables only.
#[async_trait]
pub trait ResourceGuard: Send + Sync + 'static {
    async fn before_create(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        ctx: &CreateContext,
    ) -> Result<(), GuardError>;

    async fn after_delete(
        &self,
        _tx: &mut Transaction<'_, Postgres>,
        _ctx: &DeleteContext,
    ) -> Result<(), GuardError> {
        Ok(())
    }
}

pub struct AllowAll;

#[async_trait]
impl ResourceGuard for AllowAll {
    async fn before_create(
        &self,
        _tx: &mut Transaction<'_, Postgres>,
        _ctx: &CreateContext,
    ) -> Result<(), GuardError> {
        Ok(())
    }
}
