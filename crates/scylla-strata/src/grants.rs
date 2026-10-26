use crate::access::CallerOf;
use scylla_auth::authz::{
    AuthzEntityProvider, Grant, GrantRepository, Principal, PrincipalKind, Role, RoleRepository,
    Scope, ScopeKind, check_grantable, ensure_owner_remains,
};
use scylla_core::application::agent::AgentDispatch;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{GrantId, OrganizationId};
use scylla_domain::domain::permission::{Permission, ResourceRef};
use scylla_domain::domain::role::RoleName;
use scylla_extension::Access;
use std::sync::Arc;

/// Writes a new grant. An edition overrides it to admit the grant in the same store
/// transaction as the write (for example a seat cap under a lock of the organization row).
#[strata::port]
pub trait GrantAdmission: Send + Sync + 'static {
    async fn create(&self, grants: &dyn GrantRepository, grant: &Grant) -> DomainResult<Grant>;
}

#[derive(Default)]
pub struct AdmitAll;

impl GrantAdmission for AdmitAll {
    async fn create(&self, grants: &dyn GrantRepository, grant: &Grant) -> DomainResult<Grant> {
        grants.create(grant).await
    }
}

#[strata::module(section = "grants")]
pub trait Grants: crate::Access {
    #[component]
    type GrantRepo = Arc<dyn GrantRepository>;

    #[component]
    type RoleRepo = Arc<dyn RoleRepository>;

    #[component]
    type Registry = Arc<dyn AgentDispatch>;

    #[component]
    type EntityProvider = Arc<dyn AuthzEntityProvider>;

    #[component]
    type Admission: GrantAdmission = AdmitAll;

    async fn scope_organization(&self, scope: &Scope) -> DomainResult<Option<OrganizationId>> {
        match scope {
            Scope::System => Ok(None),
            Scope::Organization(id) => Ok(Some(id.clone())),
            Scope::Project(id) => self
                .entity_provider()
                .resource_ancestors(&ResourceRef::Project(id.clone()))
                .await?
                .organization
                .map(Some)
                .ok_or_else(DomainError::missing_reference),
        }
    }

    async fn require_grantee_in(
        &self,
        grants: &[Grant],
        principal: &Principal,
        scope: &Scope,
        organization: Option<&OrganizationId>,
    ) -> DomainResult<()> {
        let admitted = match (principal, organization) {
            (_, None) => true,
            (Principal::App(id), Some(organization)) => self
                .entity_provider()
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

    #[usecase(write, access = Access::Requires(scope.manage_permission()))]
    async fn create_grant(
        &self,
        principal: Principal,
        role: RoleName,
        scope: Scope,
    ) -> DomainResult<Grant> {
        let organization = self.scope_organization(&scope).await?;
        let grants = self.grant_repo().list_all().await?;
        check_grantable(
            &self.role_repo().list_all().await?,
            &grants,
            self.ctx().caller(),
            principal.kind(),
            &role,
            &scope,
            organization.as_ref(),
        )?;
        self.require_grantee_in(&grants, &principal, &scope, organization.as_ref())
            .await?;
        let grant = self
            .admission()
            .create(
                self.grant_repo().as_ref(),
                &Grant::new(principal, role, scope),
            )
            .await?;
        if let Principal::App(app_id) = &grant.principal {
            self.registry().wake(Some(app_id));
        }
        Ok(grant)
    }

    /// The rows go in one statement, by principal and scope. The return is the number of grants
    /// removed.
    #[usecase(write, access = Access::Requires(scope.manage_permission()))]
    async fn revoke_all_access(&self, principal: Principal, scope: Scope) -> DomainResult<u64> {
        let grants = self.grant_repo().list_all().await?;
        ensure_owner_remains(&grants, |g| {
            g.principal == principal
                && match &scope {
                    Scope::System => g.scope != Scope::System,
                    scope => &g.scope == scope,
                }
        })?;
        let removed = self.grant_repo().revoke_all(&principal, &scope).await?;
        if let Principal::App(app_id) = &principal {
            self.registry().disconnect(app_id);
        }
        Ok(removed)
    }

    /// An unknown id changes nothing and the call succeeds. The last grant of a user on an
    /// organization is its membership: its revoke removes the grants of the user under the
    /// organization too, as `revoke_all_access` does.
    #[usecase(write, access = Access::Requires(Permission::RevokeGrant(id.clone())))]
    async fn revoke_grant(&self, id: GrantId) -> DomainResult<()> {
        let grants = self.grant_repo().list_all().await?;
        let id = id.as_str();
        let Some(grant) = grants.iter().find(|g| g.id == id).cloned() else {
            return Ok(());
        };
        ensure_owner_remains(&grants, |g| g.id == id)?;
        let whole_organization = grant.principal.kind() == PrincipalKind::User
            && matches!(grant.scope, Scope::Organization(_))
            && !grants
                .iter()
                .any(|g| g.id != id && g.principal == grant.principal && g.scope == grant.scope);
        if whole_organization {
            self.grant_repo()
                .revoke_all(&grant.principal, &grant.scope)
                .await?;
        } else {
            self.grant_repo().delete(&grant.id).await?;
        }
        if let Principal::App(app_id) = &grant.principal {
            self.registry().disconnect(app_id);
        }
        Ok(())
    }

    /// Without a scope, each grant of the installation; with a scope, the grants bound at that scope.
    #[usecase(read, access = Access::Requires(scope.as_ref().map_or(Permission::ManageSystemGrants, Scope::manage_permission)))]
    async fn list_grants(&self, scope: Option<Scope>) -> DomainResult<Vec<Grant>> {
        let grants = self.grant_repo().list_all().await?;
        Ok(match &scope {
            Some(scope) => grants.into_iter().filter(|g| &g.scope == scope).collect(),
            None => grants,
        })
    }

    /// The roles that a grant can bind: the platform roles, and with an organization the roles of
    /// that organization, which its members can read.
    #[usecase(read, access = match &organization_id {
        Some(o) => Access::Requires(Permission::ReadOrganization(o.clone())),
        None => Access::Authenticated,
    })]
    async fn list_grantable_roles(
        &self,
        scope_kind: Option<ScopeKind>,
        organization_id: Option<OrganizationId>,
    ) -> DomainResult<Vec<Role>> {
        let mut roles = self.role_repo().list_all().await?;
        roles.retain(|r| {
            r.usable_in(organization_id.as_ref()) && scope_kind.is_none_or(|k| r.scope == k)
        });
        Ok(roles)
    }
}
