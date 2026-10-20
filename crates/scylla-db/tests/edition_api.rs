//! The building blocks of an edition outside this repository. This file is a crate of its own: it
//! sees only the public API, as the Enterprise crates do.

use async_trait::async_trait;
use scylla_auth::authz::{
    Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, ORGANIZATION_AGENT_ROLE,
    ORGANIZATION_MEMBER_ROLE, ORGANIZATION_VIEWER_ROLE, Principal, PrincipalKind, RoleRepository,
    Scope, ScopeKind, check_grantable, grantable_role,
};
use scylla_core::application::agent::{AgentDispatch, AgentStream, JobDispatch};
use scylla_core::application::grant::CreateGrant;
use scylla_core::application::{
    GrantUseCases, NewAccount, NoOrganization, OrganizationRepository, UserRepository,
};
use scylla_core::test_support::authz::{RecordingPermissionService, actions_with};
use scylla_core::test_support::prelude::{org, user};
use scylla_db::domain::caller::CallerContext;
use scylla_db::domain::errors::{DomainError, DomainResult};
use scylla_db::domain::ids::{AppId, JobId, OrganizationId};
use scylla_db::domain::role::RoleName;
use scylla_db::domain::user::User;
use scylla_db::postgres::signup::provision_user_in;
use scylla_db::{
    PgAuthzEntityProvider, PgGrantRepository, PgOrganizationRepository, PgRoleRepository,
    PgUserRepository,
};
use scylla_extension::{Authorized, Gate, Hooks, Prepare};
use sqlx::PgPool;
use std::sync::Arc;

fn role(name: &str) -> RoleName {
    RoleName::new(name).unwrap()
}

async fn seed_user(pool: &PgPool, name: &str) -> User {
    let user = user(name);
    PgUserRepository::new(pool.clone())
        .create(&user)
        .await
        .unwrap();
    user
}

async fn seed_org(pool: &PgPool, name: &str) -> OrganizationId {
    let organization = org(name);
    PgOrganizationRepository::new(pool.clone())
        .create(&organization)
        .await
        .unwrap();
    organization.id().clone()
}

async fn grant_on(pool: &PgPool, user: &User, name: &str, organization: &OrganizationId) {
    PgGrantRepository::new(pool.clone())
        .create(&Grant::new(
            Principal::User(user.id().clone()),
            role(name),
            Scope::Organization(organization.clone()),
        ))
        .await
        .unwrap();
}

/// No agent is connected: a grant to a user wakes nothing.
struct NoAgents;

impl AgentDispatch for NoAgents {
    fn connected(&self) -> Vec<AgentStream> {
        Vec::new()
    }
    fn run(&self, _: &AgentStream, _: JobDispatch) -> DomainResult<()> {
        unreachable!("a grant runs no job")
    }
    fn cancel(&self, _: &AppId, _: &JobId) -> DomainResult<()> {
        unreachable!("a grant cancels no job")
    }
    fn disconnect(&self, _: &AppId) {}
    fn wake(&self, _: Option<&AppId>) {}
}

/// A seat limit: a new member of an organization needs a free seat; a member gets more roles.
struct MemberLimit {
    organizations: Arc<dyn OrganizationRepository>,
    seats: u64,
}

#[async_trait]
impl Gate<Prepare<CreateGrant>> for MemberLimit {
    async fn check(&self, input: &Authorized<CreateGrant>) -> DomainResult<()> {
        let cmd = input.command();
        let (Principal::User(user), Scope::Organization(organization)) =
            (&cmd.principal, &cmd.scope)
        else {
            return Ok(());
        };
        if self.organizations.has_member(organization, user).await? {
            return Ok(());
        }
        if self.organizations.count_members(organization).await? >= self.seats {
            return Err(DomainError::quota_exceeded("no free seat"));
        }
        Ok(())
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_gate_on_the_prepare_of_create_grant_sees_the_grantee_and_the_members(pool: PgPool) {
    let acme = seed_org(&pool, "Acme").await;
    let admin = seed_user(&pool, "admin").await;
    let member = seed_user(&pool, "member").await;
    let newcomer = seed_user(&pool, "newcomer").await;
    grant_on(&pool, &admin, ORGANIZATION_ADMIN_ROLE, &acme).await;
    grant_on(&pool, &member, ORGANIZATION_MEMBER_ROLE, &acme).await;

    let organizations = Arc::new(PgOrganizationRepository::new(pool.clone()));
    let mut hooks = Hooks::new();
    hooks.gate::<Prepare<CreateGrant>>(Arc::new(MemberLimit {
        organizations: organizations.clone(),
        seats: 2,
    }));
    let actions = actions_with(Arc::new(RecordingPermissionService::new()), hooks);
    let grants = GrantUseCases::new(
        Arc::new(PgGrantRepository::new(pool.clone())),
        Arc::new(PgRoleRepository::new(pool.clone())),
        Arc::new(NoAgents),
        Arc::new(PgAuthzEntityProvider::new(pool.clone())),
    );
    let caller = CallerContext::User(admin.id().clone());
    let grant = |user: &User, name: &str| CreateGrant {
        principal: Principal::User(user.id().clone()),
        role: role(name),
        scope: Scope::Organization(acme.clone()),
    };

    let refused = actions
        .run(&grants, &caller, grant(&newcomer, ORGANIZATION_MEMBER_ROLE))
        .await;
    assert!(
        matches!(refused, Err(DomainError::QuotaExceeded(_))),
        "{refused:?}"
    );
    assert!(
        !organizations
            .has_member(&acme, newcomer.id())
            .await
            .unwrap()
    );

    actions
        .run(&grants, &caller, grant(&member, ORGANIZATION_VIEWER_ROLE))
        .await
        .expect("a member gets one more role");
    assert_eq!(organizations.count_members(&acme).await.unwrap(), 2);
}

#[sqlx::test(migrations = "../../migrations")]
async fn check_grantable_runs_on_the_stored_roles_and_grants_of_an_organization(pool: PgPool) {
    let acme = seed_org(&pool, "Acme").await;
    let globex = seed_org(&pool, "Globex").await;
    let admin = seed_user(&pool, "admin").await;
    grant_on(&pool, &admin, ORGANIZATION_ADMIN_ROLE, &acme).await;

    let roles = PgRoleRepository::new(pool.clone())
        .list_all()
        .await
        .unwrap();
    let grants = PgGrantRepository::new(pool.clone())
        .list_all()
        .await
        .unwrap();
    let caller = CallerContext::User(admin.id().clone());
    let check = |organization: &OrganizationId, name: &str| {
        check_grantable(
            &roles,
            &grants,
            &caller,
            PrincipalKind::User,
            &role(name),
            &Scope::Organization(organization.clone()),
            Some(organization),
        )
    };

    check(&acme, ORGANIZATION_MEMBER_ROLE).expect("an admin gives a role of its organization");
    assert!(matches!(
        check(&globex, ORGANIZATION_MEMBER_ROLE),
        Err(DomainError::BusinessRule(_))
    ));
    assert!(matches!(
        check(&acme, ORGANIZATION_AGENT_ROLE),
        Err(DomainError::Validation(_))
    ));

    let found = grantable_role(
        &roles,
        PrincipalKind::User,
        &role(ORGANIZATION_MEMBER_ROLE),
        &Scope::Organization(acme.clone()),
        Some(&acme),
    )
    .expect("the role exists for a user on an organization");
    assert_eq!(found.scope, ScopeKind::Organization);
}

#[sqlx::test(migrations = "../../migrations")]
async fn provision_user_in_writes_in_the_transaction_of_the_edition(pool: PgPool) {
    let acme = seed_org(&pool, "Acme").await;
    sqlx::query("CREATE TABLE edition_invitations (id TEXT PRIMARY KEY, accepted_by TEXT)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO edition_invitations (id) VALUES ('invitation')")
        .execute(&pool)
        .await
        .unwrap();
    let organizations = PgOrganizationRepository::new(pool.clone());
    let users = PgUserRepository::new(pool.clone());
    let accepted_by = async || -> Option<String> {
        sqlx::query_scalar("SELECT accepted_by FROM edition_invitations")
            .fetch_one(&pool)
            .await
            .unwrap()
    };

    let accept = async |account: &NewAccount<NoOrganization>, commit: bool| -> DomainResult<()> {
        let mut tx = pool.begin().await.unwrap();
        provision_user_in(&mut tx, &account.user, &account.grants).await?;
        sqlx::query("UPDATE edition_invitations SET accepted_by = $1")
            .bind(account.user.id().as_str())
            .execute(&mut *tx)
            .await
            .unwrap();
        if commit {
            tx.commit().await.unwrap();
        } else {
            tx.rollback().await.unwrap();
        }
        Ok(())
    };
    let account = |name: &str| {
        NewAccount::without_organization(user(name)).with_grant(
            role(ORGANIZATION_MEMBER_ROLE),
            Scope::Organization(acme.clone()),
        )
    };

    let rolled_back = account("dana");
    accept(&rolled_back, false).await.unwrap();
    assert!(matches!(
        users.find_by_id(rolled_back.user.id()).await,
        Err(DomainError::NotFound(_))
    ));
    assert_eq!(organizations.count_members(&acme).await.unwrap(), 0);
    assert_eq!(accepted_by().await, None);

    let committed = account("erin");
    accept(&committed, true).await.unwrap();
    users.find_by_id(committed.user.id()).await.unwrap();
    assert!(
        organizations
            .has_member(&acme, committed.user.id())
            .await
            .unwrap()
    );
    assert_eq!(
        accepted_by().await,
        Some(committed.user.id().as_str().to_string())
    );

    let taken = accept(&account("erin"), true).await;
    assert!(matches!(taken, Err(DomainError::Conflict(_))), "{taken:?}");
    assert_eq!(organizations.count_members(&acme).await.unwrap(), 1);
}
