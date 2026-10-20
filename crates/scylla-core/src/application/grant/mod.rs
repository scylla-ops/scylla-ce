pub mod access;
pub mod commands;
pub mod queries;

pub use access::PrincipalAccess;
pub use commands::{CreateGrant, Revocation, RevokeAllAccess, RevokeGrant};
pub use queries::{ListGrantableRoles, ListGrants};

use crate::application::agent::dispatch_port::AgentDispatch;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::OrganizationId;
use crate::domain::permission::ResourceRef;
use derive_more::Constructor;
use scylla_auth::authz::{
    AuthzEntityProvider, Grant, GrantRepository, Principal, RoleRepository, Scope,
};
use std::sync::Arc;

/// The grant aggregate's stage runners, one block per action in `commands.rs` and `queries.rs`.
/// `Actions::run` drives it; the methods below are the checks of `CreateGrant`.
#[derive(Constructor)]
pub struct GrantUseCases {
    pub(super) grant_repo: Arc<dyn GrantRepository>,
    pub(super) role_repo: Arc<dyn RoleRepository>,
    pub(super) registry: Arc<dyn AgentDispatch>,
    pub(super) entity_provider: Arc<dyn AuthzEntityProvider>,
}

impl GrantUseCases {
    pub(super) async fn scope_organization(
        &self,
        scope: &Scope,
    ) -> DomainResult<Option<OrganizationId>> {
        match scope {
            Scope::System => Ok(None),
            Scope::Organization(id) => Ok(Some(id.clone())),
            Scope::Project(id) => self
                .entity_provider
                .resource_ancestors(&ResourceRef::Project(id.clone()))
                .await?
                .organization
                .map(Some)
                .ok_or_else(DomainError::missing_reference),
        }
    }

    /// The tenant boundary: an app acts only in the organization that owns it, and a user gets a
    /// project grant only once the organization admitted it. An unknown app is left to the store.
    pub(super) async fn require_grantee_in(
        &self,
        grants: &[Grant],
        principal: &Principal,
        scope: &Scope,
        organization: Option<&OrganizationId>,
    ) -> DomainResult<()> {
        let admitted = match (principal, organization) {
            (_, None) => true,
            (Principal::App(id), Some(organization)) => self
                .entity_provider
                .resource_ancestors(&ResourceRef::App(id.clone()))
                .await?
                .organization
                .is_none_or(|owner| &owner == organization),
            (Principal::User(_), Some(organization)) => {
                !matches!(scope, Scope::Project(_))
                    || grants.iter().any(|g| {
                        &g.principal == principal
                            && matches!(&g.scope, Scope::Organization(o) if o == organization)
                    })
            }
        };
        if admitted {
            Ok(())
        } else {
            Err(DomainError::business_rule(
                "the grantee must belong to this organization first",
            ))
        }
    }
}

#[cfg(test)]
mod tests;
