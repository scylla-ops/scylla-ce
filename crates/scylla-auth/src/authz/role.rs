use crate::authz::grant::{Scope, ScopeKind};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::OrganizationId;
use crate::domain::permission::permission_resource_type;
use crate::domain::role::{RoleDescription, RoleDisplayName};
use async_trait::async_trait;
use std::collections::HashMap;

/// Unconstrained Cedar action: an admin role covers permissions added later without a re-seed.
pub const FULL_CONTROL: &str = "*";

/// `Admin` is for the builtin owner roles only; `Agent` roles go to apps only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleKind {
    Admin,
    Member,
    Agent,
}

impl RoleKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
            Self::Agent => "agent",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "admin" => Some(Self::Admin),
            "member" => Some(Self::Member),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }
}

/// A role with no owner is a platform role; a role with an owner belongs to that organization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub id: String,
    pub key: Option<String>,
    pub name: RoleDisplayName,
    pub description: RoleDescription,
    pub scope: ScopeKind,
    pub kind: RoleKind,
    pub owner_org: Option<OrganizationId>,
    pub builtin: bool,
    pub permissions: Vec<String>,
    pub version: u64,
}

impl Role {
    #[must_use]
    pub fn is_full_control(&self) -> bool {
        self.permissions.iter().any(|p| p == FULL_CONTROL)
    }

    /// Where the role may be seen and granted: a platform role everywhere, an organization role
    /// only in its organization.
    #[must_use]
    pub fn usable_in(&self, organization: Option<&OrganizationId>) -> bool {
        self.owner_org.is_none() || self.owner_org.as_ref() == organization
    }

    #[must_use]
    pub fn new_custom(
        name: RoleDisplayName,
        description: RoleDescription,
        scope: ScopeKind,
        kind: RoleKind,
        owner_org: Option<OrganizationId>,
        permissions: Vec<String>,
    ) -> Self {
        Self {
            id: scylla_domain::domain::ids::new_id(),
            key: None,
            name,
            description,
            scope,
            kind,
            owner_org,
            builtin: false,
            permissions,
            version: 0,
        }
    }
}

pub fn resource_home_scope(resource_type: &str) -> ScopeKind {
    match resource_type {
        "organization" | "app" | "app_secret" => ScopeKind::Organization,
        "project" | "pipeline" | "job" | "secret" | "trigger" => ScopeKind::Project,
        _ => ScopeKind::System,
    }
}

pub fn validate_role_permissions(permissions: &[String], scope: ScopeKind) -> DomainResult<()> {
    if permissions.is_empty() {
        return Err(DomainError::validation(
            "a role must have at least one permission",
        ));
    }
    for p in permissions {
        if p == FULL_CONTROL {
            continue;
        }
        let Some(resource_type) = permission_resource_type(p) else {
            return Err(DomainError::validation(format!("unknown permission '{p}'")));
        };
        let home = resource_home_scope(resource_type);
        if !scope.covers(home) {
            return Err(DomainError::validation(format!(
                "permission '{p}' targets the resource type '{resource_type}': a role of scope \
                 '{}' cannot hold it; use a role of scope '{}' or broader",
                scope.as_str(),
                home.as_str()
            )));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveScope {
    pub scope: Scope,
    pub full_control: bool,
    pub permissions: Vec<String>,
}

pub const ROLE_IN_USE: &str = "role is still granted; revoke those grants first";

#[async_trait]
pub trait RoleRepository: Send + Sync {
    async fn list_all(&self) -> DomainResult<Vec<Role>>;
    async fn get(&self, id: &str) -> DomainResult<Option<Role>>;
    async fn create(&self, role: &Role) -> DomainResult<()>;
    async fn update(&self, role: &Role) -> DomainResult<()>;
    /// A business rule error, `ROLE_IN_USE`, when a grant names the role at the time of the
    /// delete.
    async fn delete(&self, role: &Role) -> DomainResult<()>;
    /// Granted.
    async fn in_use(&self, id: &str) -> DomainResult<bool>;
}

/// Every role's permission keys, by role id.
pub async fn permissions_by_role(
    repo: &dyn RoleRepository,
) -> DomainResult<HashMap<String, Vec<String>>> {
    Ok(repo
        .list_all()
        .await?
        .into_iter()
        .map(|r| (r.id, r.permissions))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{FULL_CONTROL, ScopeKind, validate_role_permissions};

    #[test]
    fn validate_role_permissions_accepts_keys_and_wildcard_rejects_others() {
        assert!(
            validate_role_permissions(&["runPipeline".into(), "readJob".into()], ScopeKind::System)
                .is_ok()
        );
        assert!(validate_role_permissions(&[FULL_CONTROL.into()], ScopeKind::Project).is_ok());
        assert!(validate_role_permissions(&[], ScopeKind::System).is_err());
        assert!(validate_role_permissions(&["flyToTheMoon".into()], ScopeKind::System).is_err());
    }

    #[test]
    fn validate_role_permissions_enforces_scope_coherence() {
        assert!(
            validate_role_permissions(&["createOrganization".into()], ScopeKind::System).is_ok()
        );
        assert!(
            validate_role_permissions(&["createOrganization".into()], ScopeKind::Organization)
                .is_err()
        );
        assert!(
            validate_role_permissions(&["createOrganization".into()], ScopeKind::Project).is_err()
        );

        assert!(
            validate_role_permissions(&["createProject".into()], ScopeKind::Organization).is_ok()
        );
        assert!(validate_role_permissions(&["createProject".into()], ScopeKind::Project).is_err());

        assert!(validate_role_permissions(&["runPipeline".into()], ScopeKind::Project).is_ok());
        assert!(
            validate_role_permissions(&["runPipeline".into()], ScopeKind::Organization).is_ok()
        );
        assert!(validate_role_permissions(&["runPipeline".into()], ScopeKind::System).is_ok());
    }
}
