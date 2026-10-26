pub mod account;
pub mod repository;
pub mod reset;

pub use account::{AccountRepository, UserSession};
pub use repository::{UserRepository, users_in_order};
pub use reset::{
    PasswordResetDelivery, PasswordResetMessage, PasswordResetSender, PasswordResetUseCases,
};

use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{SessionId, UserId};
use crate::domain::user::User;
use scylla_auth::authz::{
    Grant, GrantRepository, Principal, SYSTEM_ADMIN_ROLE, Scope, ensure_owner_remains,
};
use std::sync::Arc;

pub const WRONG_CURRENT_PASSWORD: &str = "The current password is wrong";
pub const WRONG_PASSWORD: &str = "The password is wrong";

/// Not `Unauthorized`: the web UI signs out on UNAUTHENTICATED.
#[must_use]
pub fn wrong_current_password() -> DomainError {
    DomainError::business_rule(WRONG_CURRENT_PASSWORD)
}

#[must_use]
pub fn wrong_password() -> DomainError {
    DomainError::business_rule(WRONG_PASSWORD)
}

/// One error for an unknown id, a session of another user and an expired session.
pub fn session_not_found(id: &SessionId) -> DomainError {
    DomainError::not_found("Session", id)
}

/// The user rules that two use cases or more of the Users module of strata read.
pub struct UserUseCases {
    pub(super) user_repo: Arc<dyn UserRepository>,
    pub(super) grant_repo: Arc<dyn GrantRepository>,
    pub(super) accounts: Arc<dyn AccountRepository>,
}

impl UserUseCases {
    pub fn new(
        user_repo: Arc<dyn UserRepository>,
        grant_repo: Arc<dyn GrantRepository>,
        accounts: Arc<dyn AccountRepository>,
    ) -> Self {
        Self {
            user_repo,
            grant_repo,
            accounts,
        }
    }

    /// A user that holds `system-admin` leaves the active users only when another active user
    /// holds it.
    pub async fn ensure_an_active_admin_remains(&self, leaving: &UserId) -> DomainResult<()> {
        let grants = self.grant_repo.list_all().await?;
        let admin = |g: &Grant| g.scope == Scope::System && g.role.as_str() == SYSTEM_ADMIN_ROLE;
        let leaving_principal = Principal::User(leaving.clone());
        if !grants
            .iter()
            .any(|g| admin(g) && g.principal == leaving_principal)
        {
            return Ok(());
        }
        let others: Vec<UserId> = grants
            .iter()
            .filter(|g| admin(g))
            .filter_map(|g| match &g.principal {
                Principal::User(id) if id != leaving => Some(id.clone()),
                Principal::User(_) | Principal::App(_) => None,
            })
            .collect();
        if self
            .user_repo
            .find_by_ids(&others)
            .await?
            .iter()
            .any(User::is_active)
        {
            return Ok(());
        }
        Err(DomainError::business_rule(
            "cannot deactivate the last active system administrator: appoint another first",
        ))
    }

    /// The rule of `ensure_owner_remains`, with a message that names the organizations.
    pub async fn ensure_owner_remains_without(&self, user: &User) -> DomainResult<()> {
        let grants = self.grant_repo.list_all().await?;
        let principal = Principal::User(user.id().clone());
        let mut orphaned: Vec<&Scope> = Vec::new();
        for scope in grants
            .iter()
            .filter(|g| g.principal == principal)
            .map(|g| &g.scope)
        {
            let last =
                ensure_owner_remains(&grants, |o| o.principal == principal && &o.scope == scope)
                    .is_err();
            if last && !orphaned.contains(&scope) {
                orphaned.push(scope);
            }
        }
        if orphaned.is_empty() {
            return Ok(());
        }
        let mut roles = Vec::new();
        if orphaned.contains(&&Scope::System) {
            roles.push("the last system administrator".to_owned());
        }
        let organizations: Vec<_> = orphaned
            .iter()
            .filter_map(|scope| match scope {
                Scope::Organization(id) => Some(id),
                Scope::System | Scope::Project(_) => None,
            })
            .collect();
        if !organizations.is_empty() {
            let access = self.accounts.list_access(user.id()).await?;
            let mut names: Vec<String> = organizations
                .iter()
                .map(|id| {
                    access
                        .iter()
                        .find(|a| a.organization_id.as_ref() == Some(*id))
                        .and_then(|a| a.organization_name.as_ref())
                        .map_or_else(|| id.to_string(), ToString::to_string)
                })
                .collect();
            names.sort();
            roles.push(format!(
                "the last organization administrator of {}",
                names.join(", ")
            ));
        }
        Err(DomainError::business_rule(format!(
            "Cannot delete your account: you are {}. Appoint another administrator first.",
            roles.join(" and ")
        )))
    }
}

#[cfg(test)]
mod tests;
