//! The pipeline's authorizer is the access model's permission check. An adapter, not a blanket
//! impl: `Authorizer` and `PermissionService` are both foreign here (E0210).

use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, UserId};
use crate::domain::job::JobOrigin;
use crate::domain::permission::Permission;
use async_trait::async_trait;
use scylla_auth::authz::PermissionService;
use scylla_extension::Authorizer;
use std::sync::Arc;

pub struct PermissionAuthorizer {
    permissions: Arc<dyn PermissionService>,
}

impl PermissionAuthorizer {
    #[must_use]
    pub fn new(permissions: Arc<dyn PermissionService>) -> Self {
        Self { permissions }
    }
}

#[async_trait]
impl Authorizer for PermissionAuthorizer {
    async fn authorize(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        self.permissions.check(caller, permission).await
    }
}

/// The guard of a pass over many rows: no resource for Cedar to decide on, so the action is
/// `Authenticated` and its `Prepare` refuses every caller but an in-process service.
pub(crate) fn service_only(caller: &CallerContext) -> DomainResult<()> {
    match caller {
        CallerContext::Service(_) => Ok(()),
        _ => Err(DomainError::forbidden(
            "only an in-process service runs this pass",
        )),
    }
}

/// The guard of an agent's report on itself: no permission reaches the agent's own App, so the
/// action is `Authenticated` and the target is the caller.
pub(crate) fn app_only(caller: &CallerContext) -> DomainResult<AppId> {
    match caller {
        CallerContext::App(app_id) => Ok(app_id.clone()),
        _ => Err(DomainError::forbidden(
            "only an agent reports its own state",
        )),
    }
}

/// The guard of an action on the caller's own account: the target is the caller, and an app or a
/// service has no account.
pub(crate) fn user_only(caller: &CallerContext) -> DomainResult<UserId> {
    match caller {
        CallerContext::User(user_id) => Ok(user_id.clone()),
        _ => Err(DomainError::forbidden(
            "only a user acts on its own account",
        )),
    }
}

/// The origin of a job that a caller starts directly: a user or an app, never a service.
pub(crate) fn user_or_app(caller: &CallerContext) -> DomainResult<JobOrigin> {
    match caller {
        CallerContext::User(user_id) => Ok(JobOrigin::Human {
            user_id: user_id.clone(),
        }),
        CallerContext::App(app_id) => Ok(JobOrigin::App {
            app_id: app_id.clone(),
        }),
        CallerContext::Service(_) | CallerContext::Anonymous => Err(DomainError::forbidden(
            "only a user or app can run a pipeline directly",
        )),
    }
}
