pub mod commands;
pub mod queries;

pub use commands::{CreateRole, DeleteRole, UpdateRole};
pub use queries::{
    GetEffectivePermissions, GetMyPermissions, GetRole, ListAuthzVocabulary, ListRoles,
};

use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::OrganizationId;
use crate::domain::role::RoleName;
use derive_more::Constructor;
use scylla_auth::authz::{
    EffectiveScope, FULL_CONTROL, GrantRepository, Principal, Role, RoleRepository, Scope,
    ensure_no_escalation, permissions_by_role,
};
use std::collections::BTreeSet;
use std::sync::Arc;

/// The role aggregate's stage runners, one block per action in `commands.rs` and `queries.rs`.
#[derive(Constructor)]
pub struct RoleUseCases {
    pub(super) role_repo: Arc<dyn RoleRepository>,
    pub(super) grant_repo: Arc<dyn GrantRepository>,
}

impl RoleUseCases {
    pub(super) async fn role(&self, id: &RoleName) -> DomainResult<Role> {
        self.role_repo
            .get(id.as_str())
            .await?
            .ok_or_else(|| DomainError::not_found("Role", id))
    }

    /// The caller holds every permission it puts in a role of `organization`, or of the platform.
    pub(super) async fn ensure_no_escalation(
        &self,
        caller: &CallerContext,
        permissions: &[String],
        organization: Option<&OrganizationId>,
    ) -> DomainResult<()> {
        let scope = organization.map_or(Scope::System, |o| Scope::Organization(o.clone()));
        let roles = self.role_repo.list_all().await?;
        let grants = self.grant_repo.list_all().await?;
        ensure_no_escalation(&roles, &grants, caller, permissions, &scope, organization)
    }

    /// Grants as bound per scope: a System grant is not re-listed under every org and project.
    pub(super) async fn effective_scopes(
        &self,
        principal: &Principal,
    ) -> DomainResult<Vec<EffectiveScope>> {
        let role_perms = permissions_by_role(self.role_repo.as_ref()).await?;
        let grants = self.grant_repo.list_all().await?;

        let mut groups: Vec<(Scope, BTreeSet<String>)> = Vec::new();
        for grant in grants.iter().filter(|g| g.principal == *principal) {
            let perms: Vec<String> = role_perms
                .get(grant.role.as_str())
                .cloned()
                .unwrap_or_default();
            if let Some(idx) = groups.iter().position(|(s, _)| *s == grant.scope) {
                groups[idx].1.extend(perms);
            } else {
                groups.push((grant.scope.clone(), perms.into_iter().collect()));
            }
        }

        Ok(groups
            .into_iter()
            .map(|(scope, set)| {
                let full_control = set.contains(FULL_CONTROL);
                EffectiveScope {
                    scope,
                    full_control,
                    permissions: if full_control {
                        Vec::new()
                    } else {
                        set.into_iter().collect()
                    },
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests;
