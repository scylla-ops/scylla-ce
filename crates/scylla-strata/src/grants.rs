use crate::access::CallerOf;
use scylla_auth::authz::{
    AuthzEntityProvider, Grant, GrantRepository, Principal, PrincipalKind, Role, RoleRepository,
    Scope, check_grantable, ensure_owner_remains,
};
use scylla_core::application::agent::AgentDispatch;
use scylla_core::application::grant::{
    CreateGrant, ListGrantableRoles, ListGrants, RevokeAllAccess, RevokeGrant,
};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::OrganizationId;
use scylla_domain::domain::permission::ResourceRef;
use scylla_extension::Describe;
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

    #[usecase(write, access = cmd.access())]
    async fn create_grant(&self, cmd: CreateGrant) -> DomainResult<Grant> {
        let organization = self.scope_organization(&cmd.scope).await?;
        let grants = self.grant_repo().list_all().await?;
        check_grantable(
            &self.role_repo().list_all().await?,
            &grants,
            self.ctx().caller(),
            cmd.principal.kind(),
            &cmd.role,
            &cmd.scope,
            organization.as_ref(),
        )?;
        self.require_grantee_in(&grants, &cmd.principal, &cmd.scope, organization.as_ref())
            .await?;
        let grant = self
            .admission()
            .create(
                self.grant_repo().as_ref(),
                &Grant::new(cmd.principal, cmd.role, cmd.scope),
            )
            .await?;
        if let Principal::App(app_id) = &grant.principal {
            self.registry().wake(Some(app_id));
        }
        Ok(grant)
    }

    #[usecase(write, access = cmd.access())]
    async fn revoke_all_access(&self, cmd: RevokeAllAccess) -> DomainResult<u64> {
        let grants = self.grant_repo().list_all().await?;
        ensure_owner_remains(&grants, |g| {
            g.principal == cmd.principal
                && match &cmd.scope {
                    Scope::System => g.scope != Scope::System,
                    scope => &g.scope == scope,
                }
        })?;
        let removed = self
            .grant_repo()
            .revoke_all(&cmd.principal, &cmd.scope)
            .await?;
        if let Principal::App(app_id) = &cmd.principal {
            self.registry().disconnect(app_id);
        }
        Ok(removed)
    }

    #[usecase(write, access = cmd.access())]
    async fn revoke_grant(&self, cmd: RevokeGrant) -> DomainResult<()> {
        let grants = self.grant_repo().list_all().await?;
        let id = cmd.id.as_str();
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

    #[usecase(read, access = query.access())]
    async fn list_grants(&self, query: ListGrants) -> DomainResult<Vec<Grant>> {
        let grants = self.grant_repo().list_all().await?;
        Ok(match &query.scope {
            Some(scope) => grants.into_iter().filter(|g| &g.scope == scope).collect(),
            None => grants,
        })
    }

    #[usecase(read, access = query.access())]
    async fn list_grantable_roles(&self, query: ListGrantableRoles) -> DomainResult<Vec<Role>> {
        let mut roles = self.role_repo().list_all().await?;
        roles.retain(|r| {
            r.usable_in(query.organization_id.as_ref())
                && query.scope_kind.is_none_or(|k| r.scope == k)
        });
        Ok(roles)
    }
}
