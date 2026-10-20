use crate::domain::caller::CallerContext;
use crate::domain::errors::DomainResult;
use crate::domain::permission::Permission;

/// `Err(Forbidden)` and never `Ok(false)`: a denial cannot be mistaken for success.
#[async_trait::async_trait]
pub trait PermissionService: Send + Sync {
    async fn check(&self, caller: &CallerContext, perm: Permission) -> DomainResult<()>;
}
