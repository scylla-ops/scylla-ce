//! The grant's writes. One block per command, in the order it runs: the struct, its access,
//! its payload types, what `Prepare` checks and builds, what `Persist` writes.

use super::GrantUseCases;
use crate::domain::errors::DomainResult;
use crate::domain::ids::GrantId;
use crate::domain::permission::Permission;
use crate::domain::role::RoleName;
use async_trait::async_trait;
use scylla_auth::authz::{
    Grant, Principal, PrincipalKind, Scope, check_grantable, ensure_owner_remains,
};
use scylla_extension::{
    Access, Authorized, Command, Committed, Deleted, Describe, Draft, Persist, Prepare, Prepared,
    Run,
};

#[derive(Debug)]
pub struct CreateGrant {
    pub principal: Principal,
    pub role: RoleName,
    pub scope: Scope,
}

impl Describe for CreateGrant {
    fn access(&self) -> Access {
        Access::Requires(self.scope.manage_permission())
    }
}

impl Command for CreateGrant {
    type Staged = Draft<Grant>;
    type Committed = Grant;
}

#[async_trait]
impl Run<Prepare<CreateGrant>> for GrantUseCases {
    async fn run(&self, input: Authorized<CreateGrant>) -> DomainResult<Prepared<CreateGrant>> {
        let cmd = input.command();
        let organization = self.scope_organization(&cmd.scope).await?;
        let grants = self.grant_repo.list_all().await?;
        check_grantable(
            &self.role_repo.list_all().await?,
            &grants,
            input.caller(),
            cmd.principal.kind(),
            &cmd.role,
            &cmd.scope,
            organization.as_ref(),
        )?;
        self.require_grantee_in(&grants, &cmd.principal, &cmd.scope, organization.as_ref())
            .await?;
        let grant = Grant::new(cmd.principal.clone(), cmd.role.clone(), cmd.scope.clone());
        Ok(input.prepared(Draft::new(grant)))
    }
}

#[async_trait]
impl Run<Persist<CreateGrant>> for GrantUseCases {
    async fn run(&self, input: Prepared<CreateGrant>) -> DomainResult<Committed<CreateGrant>> {
        input
            .commit(async |draft| {
                let grant = self.grant_repo.create(&draft.into_inner()).await?;
                if let Principal::App(app_id) = &grant.principal {
                    self.registry.wake(Some(app_id));
                }
                Ok(grant)
            })
            .await
    }
}

#[derive(Debug)]
pub struct RevokeAllAccess {
    pub principal: Principal,
    pub scope: Scope,
}

impl Describe for RevokeAllAccess {
    fn access(&self) -> Access {
        Access::Requires(self.scope.manage_permission())
    }
}

/// Stages the principal alone: the rows go in one statement, by principal and scope. `Committed`
/// is the number of grants removed.
impl Command for RevokeAllAccess {
    type Staged = Principal;
    type Committed = u64;
}

#[async_trait]
impl Run<Prepare<RevokeAllAccess>> for GrantUseCases {
    async fn run(
        &self,
        input: Authorized<RevokeAllAccess>,
    ) -> DomainResult<Prepared<RevokeAllAccess>> {
        let cmd = input.command();
        let grants = self.grant_repo.list_all().await?;
        ensure_owner_remains(&grants, |g| {
            g.principal == cmd.principal
                && match &cmd.scope {
                    Scope::System => g.scope != Scope::System,
                    scope => &g.scope == scope,
                }
        })?;
        let principal = cmd.principal.clone();
        Ok(input.prepared(principal))
    }
}

#[async_trait]
impl Run<Persist<RevokeAllAccess>> for GrantUseCases {
    async fn run(
        &self,
        input: Prepared<RevokeAllAccess>,
    ) -> DomainResult<Committed<RevokeAllAccess>> {
        let scope = input.command().scope.clone();
        input
            .commit(async |principal| {
                let removed = self.grant_repo.revoke_all(&principal, &scope).await?;
                if let Principal::App(app_id) = &principal {
                    self.registry.disconnect(app_id);
                }
                Ok(removed)
            })
            .await
    }
}

#[derive(Debug)]
pub struct RevokeGrant {
    pub id: GrantId,
}

impl Describe for RevokeGrant {
    fn access(&self) -> Access {
        Access::Requires(Permission::RevokeGrant(self.id.clone()))
    }
}

/// The last grant of a user on an organization is its membership: revoking it strips the user's
/// grants beneath the organization too, as `RevokeAllAccess` does.
#[derive(Debug)]
pub struct Revocation {
    pub grant: Grant,
    pub whole_organization: bool,
}

/// An unknown id stages `None` and commits nothing: the call succeeds without change.
impl Command for RevokeGrant {
    type Staged = Option<Revocation>;
    type Committed = Option<Deleted<Grant>>;
}

#[async_trait]
impl Run<Prepare<RevokeGrant>> for GrantUseCases {
    async fn run(&self, input: Authorized<RevokeGrant>) -> DomainResult<Prepared<RevokeGrant>> {
        let grants = self.grant_repo.list_all().await?;
        let id = input.command().id.as_str();
        let Some(grant) = grants.iter().find(|g| g.id == id).cloned() else {
            return Ok(input.prepared(None));
        };
        ensure_owner_remains(&grants, |g| g.id == id)?;
        let whole_organization = grant.principal.kind() == PrincipalKind::User
            && matches!(grant.scope, Scope::Organization(_))
            && !grants
                .iter()
                .any(|g| g.id != id && g.principal == grant.principal && g.scope == grant.scope);
        Ok(input.prepared(Some(Revocation {
            grant,
            whole_organization,
        })))
    }
}

#[async_trait]
impl Run<Persist<RevokeGrant>> for GrantUseCases {
    async fn run(&self, input: Prepared<RevokeGrant>) -> DomainResult<Committed<RevokeGrant>> {
        input
            .commit(async |revocation| {
                let Some(Revocation {
                    grant,
                    whole_organization,
                }) = revocation
                else {
                    return Ok(None);
                };
                if whole_organization {
                    self.grant_repo
                        .revoke_all(&grant.principal, &grant.scope)
                        .await?;
                } else {
                    self.grant_repo.delete(&grant.id).await?;
                }
                if let Principal::App(app_id) = &grant.principal {
                    self.registry.disconnect(app_id);
                }
                Ok(Some(Deleted::new(grant)))
            })
            .await
    }
}
