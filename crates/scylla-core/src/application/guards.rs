use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, UserId};
use crate::domain::job::JobOrigin;

/// The guard of a pass over many rows: no resource for Cedar to decide on, so the action is
/// `Authenticated` and the use case refuses every caller but an in-process service.
pub fn service_only(caller: &CallerContext) -> DomainResult<()> {
    match caller {
        CallerContext::Service(_) => Ok(()),
        _ => Err(DomainError::forbidden(
            "only an in-process service runs this pass",
        )),
    }
}

/// The guard of an agent's report on itself: no permission reaches the agent's own App, so the
/// action is `Authenticated` and the target is the caller.
pub fn app_only(caller: &CallerContext) -> DomainResult<AppId> {
    match caller {
        CallerContext::App(app_id) => Ok(app_id.clone()),
        _ => Err(DomainError::forbidden(
            "only an agent reports its own state",
        )),
    }
}

/// The guard of an action on the caller's own account: the target is the caller, and an app or a
/// service has no account.
pub fn user_only(caller: &CallerContext) -> DomainResult<UserId> {
    match caller {
        CallerContext::User(user_id) => Ok(user_id.clone()),
        _ => Err(DomainError::forbidden(
            "only a user acts on its own account",
        )),
    }
}

/// The origin of a job that a caller starts directly: a user or an app, never a service.
pub fn user_or_app(caller: &CallerContext) -> DomainResult<JobOrigin> {
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
