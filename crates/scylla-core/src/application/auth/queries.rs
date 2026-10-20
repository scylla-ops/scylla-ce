//! The session's reads. One block per query: the struct, its access, its output, what `Fetch`
//! reads.

use super::{AuthUseCases, SessionLookup, look_up_session};
use crate::domain::errors::DomainResult;
use async_trait::async_trait;
use scylla_extension::{Access, Authorized, Describe, Fetch, Fetched, Query, Run};

/// No `Debug`: `token` is the session credential.
pub struct ValidateToken {
    pub token: String,
}

impl Describe for ValidateToken {
    fn access(&self) -> Access {
        Access::Public
    }
}

impl Query for ValidateToken {
    type Output = bool;
}

/// A lookup that fails is an invalid token, not an error: the answer is `false`.
#[async_trait]
impl Run<Fetch<ValidateToken>> for AuthUseCases {
    async fn run(&self, input: Authorized<ValidateToken>) -> DomainResult<Fetched<ValidateToken>> {
        let token = input.command().token.as_str();
        let valid = if token.is_empty() {
            false
        } else {
            matches!(
                look_up_session(&*self.session_repo, token).await,
                Ok(SessionLookup::Live(_))
            )
        };
        Ok(input.fetched(valid))
    }
}
