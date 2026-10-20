//! The access check is a stage, not a hook: it runs with zero extensions registered, and its
//! output type is the proof that it ran.

use crate::action::{Authorized, Describe, Requested};
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::permission::Permission;
use crate::stage::{Authorize, Run};
use async_trait::async_trait;
use std::fmt;
use std::slice;
use std::sync::Arc;

/// Who may run an action. `Public` and `Authenticated` never reach the `Authorizer`; each
/// permission of `Requires` or `RequiresAll` does, in order, and the first refusal stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Access {
    Public,
    Authenticated,
    Requires(Permission),
    RequiresAll(Vec<Permission>),
}

impl Access {
    #[must_use]
    pub fn permissions(&self) -> &[Permission] {
        match self {
            Self::Public | Self::Authenticated => &[],
            Self::Requires(permission) => slice::from_ref(permission),
            Self::RequiresAll(permissions) => permissions,
        }
    }
}

impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Public => f.write_str("public"),
            Self::Authenticated => f.write_str("authenticated"),
            Self::Requires(_) | Self::RequiresAll(_) => {
                let keys: Vec<_> = self.permissions().iter().map(Permission::key).collect();
                f.write_str(&keys.join("+"))
            }
        }
    }
}

/// Only `AuthorizeStage` builds one, and `Requested::authorized` consumes it, so an
/// `Authorized<C>` can only come from the check.
#[derive(Debug, Clone, Copy)]
pub struct Granted(());

/// `Err(Forbidden)` and never `Ok(false)`: the `PermissionService` contract, restated here so the
/// pipeline does not depend on the access model crate.
#[async_trait]
pub trait Authorizer: Send + Sync {
    async fn authorize(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()>;
}

pub struct AuthorizeStage {
    authorizer: Arc<dyn Authorizer>,
}

impl AuthorizeStage {
    #[must_use]
    pub fn new(authorizer: Arc<dyn Authorizer>) -> Self {
        Self { authorizer }
    }
}

#[async_trait]
impl<C: Describe> Run<Authorize<C>> for AuthorizeStage {
    async fn run(&self, input: Requested<C>) -> DomainResult<Authorized<C>> {
        let access = input.access();
        if *access == Access::Authenticated && *input.caller() == CallerContext::Anonymous {
            return Err(DomainError::forbidden("Anonymous caller is not permitted"));
        }
        for permission in access.permissions() {
            self.authorizer
                .authorize(input.caller(), permission.clone())
                .await?;
        }
        Ok(input.authorized(Granted(())))
    }
}
