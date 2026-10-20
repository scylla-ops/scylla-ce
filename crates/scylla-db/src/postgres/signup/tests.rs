use super::PgSignupRepository;
use crate::domain::role::RoleName;
use crate::postgres::{
    PgGrantRepository, PgOrganizationRepository, PgRoleRepository, PgUserRepository,
};
use crate::test_support::prelude::*;
use scylla_auth::authz::{Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, Principal, Scope};
use scylla_core::application::signup::repository::SignupRepository;
use scylla_core::application::{NewAccount, OrganizationRepository, UserRepository};
use sqlx::PgPool;

fn org_admin_grant(
    user_id: crate::domain::ids::UserId,
    org_id: crate::domain::ids::OrganizationId,
) -> Grant {
    Grant::new(
        Principal::User(user_id),
        RoleName::new(ORGANIZATION_ADMIN_ROLE).unwrap(),
        Scope::Organization(org_id),
    )
}

#[sqlx::test(migrations = "../../migrations")]
async fn provision_account_persists_all_four_rows(pool: PgPool) {
    let repo = PgSignupRepository::new(pool.clone());
    let user = user("founder");
    let org = org("Acme");
    let grant = org_admin_grant(user.id().clone(), org.id().clone());

    repo.provision_account(&user, &org, std::slice::from_ref(&grant))
        .await
        .expect("provision");

    PgUserRepository::new(pool.clone())
        .find_by_id(user.id())
        .await
        .expect("user persisted");
    PgOrganizationRepository::new(pool.clone())
        .find_by_id(org.id())
        .await
        .expect("org persisted");
    let grants = PgGrantRepository::new(pool).list_all().await.unwrap();
    assert!(
        grants.iter().any(|g| g.id == grant.id),
        "org-admin grant persisted"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn username_conflict_rolls_back_the_whole_account(pool: PgPool) {
    let repo = PgSignupRepository::new(pool.clone());

    let first_user = user("dup");
    let first_org = org("FirstOrg");
    let first_grant = org_admin_grant(first_user.id().clone(), first_org.id().clone());
    repo.provision_account(&first_user, &first_org, std::slice::from_ref(&first_grant))
        .await
        .expect("first provision");

    let clash_user = UserBuilder::new("dup").build();
    let second_org = org("SecondOrg");
    let second_grant = org_admin_grant(clash_user.id().clone(), second_org.id().clone());
    let err = repo
        .provision_account(
            &clash_user,
            &second_org,
            std::slice::from_ref(&second_grant),
        )
        .await
        .expect_err("username clash must fail");
    assert!(
        matches!(err, crate::domain::errors::DomainError::Conflict(_)),
        "expected Conflict, got {err:?}"
    );

    let org_repo = PgOrganizationRepository::new(pool.clone());
    assert!(
        matches!(
            org_repo.find_by_id(second_org.id()).await,
            Err(crate::domain::errors::DomainError::NotFound(_))
        ),
        "second org must not be persisted"
    );
    let grants = PgGrantRepository::new(pool).list_all().await.unwrap();
    assert!(
        !grants.iter().any(|g| g.id == second_grant.id),
        "second grant must not be persisted"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn login_by_email_or_username(pool: PgPool) {
    use crate::domain::caller::CallerContext;
    use crate::domain::user::User;
    use crate::domain::user::{Email, Password, Username};
    use crate::postgres::PgSessionRepository;
    use scylla_core::application::auth::{AuthUseCases, Login};
    use scylla_core::application::{HashService, UserRepository};
    use scylla_core::infrastructure::Argon2HashService;
    use scylla_core::test_support::authz::DenyingPermissionService;
    use std::sync::Arc;

    let hash = Arc::new(Argon2HashService::new());
    let user_repo = Arc::new(PgUserRepository::new(pool.clone()));
    let auth = AuthUseCases::new(
        user_repo.clone(),
        Arc::new(PgSessionRepository::new(pool.clone())),
        hash.clone(),
    );

    let password_hash = hash
        .hash(&Password::new("SecurePass123!").unwrap())
        .await
        .unwrap();
    let user = User::create(
        Username::new("kevin").unwrap(),
        Some(Email::new("kevin@example.com").unwrap()),
        password_hash,
    );
    user_repo.create(&user).await.expect("seed user");

    let engine = actions(Arc::new(DenyingPermissionService::new()));
    let login = |identifier: &str, password: &str| {
        engine.run(
            &auth,
            &CallerContext::Anonymous,
            Login {
                identifier: identifier.to_string(),
                password: Password::new(password).unwrap(),
                client: crate::domain::session::SessionClient::default(),
            },
        )
    };

    login("kevin@example.com", "SecurePass123!")
        .await
        .expect("login by email");
    login("kevin", "SecurePass123!")
        .await
        .expect("login by username");
    let err = login("kevin", "WrongPass123!")
        .await
        .expect_err("wrong password rejected");
    assert!(matches!(
        err,
        crate::domain::errors::DomainError::Unauthorized(_)
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_provisioned_account_is_admin_of_its_own_org_only(pool: PgPool) {
    use crate::domain::caller::CallerContext;
    use crate::domain::organization::OrganizationName;
    use crate::domain::permission::Permission;
    use crate::domain::role::RoleName;
    use crate::postgres::PgAuthzEntityProvider;
    use scylla_auth::audit::NoopAuditLog;
    use scylla_auth::authz::{ORGANIZATION_CREATOR_ROLE, PermissionService, Scope};
    use scylla_auth::cedar::CedarPermissionService;
    use std::sync::Arc;

    let foreign = seed_org(&pool, "Foreign Corp").await;
    let account = NewAccount::new(
        user("founder"),
        OrganizationName::new("Founders Inc").unwrap(),
    )
    .unwrap()
    .with_grant(
        RoleName::new(ORGANIZATION_CREATOR_ROLE).unwrap(),
        Scope::System,
    );
    PgSignupRepository::new(pool.clone())
        .provision_account(&account.user, &account.organization, &account.grants)
        .await
        .expect("provision");

    let permission = Arc::new(
        CedarPermissionService::new(
            Arc::new(PgAuthzEntityProvider::new(pool.clone())),
            Arc::new(PgRoleRepository::new(pool.clone())),
            Arc::new(PgGrantRepository::new(pool.clone())),
            Arc::new(NoopAuditLog),
        )
        .await
        .expect("cedar service"),
    );
    let caller = CallerContext::User(account.user.id().clone());

    permission
        .check(
            &caller,
            Permission::UpdateOrganization(account.organization.id().clone()),
        )
        .await
        .expect("org-admin can update own org");
    permission
        .check(&caller, Permission::CreateOrganization)
        .await
        .expect("the organization creator role lets it create organizations");
    assert!(
        permission
            .check(&caller, Permission::ListUsers)
            .await
            .is_err()
    );

    let err = permission
        .check(
            &caller,
            Permission::UpdateOrganization(foreign.id().clone()),
        )
        .await
        .expect_err("must be denied on a foreign org");
    assert!(
        matches!(err, crate::domain::errors::DomainError::Forbidden(_)),
        "expected Forbidden on cross-tenant access, got {err:?}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_account_without_an_organization_writes_the_user_and_its_grants(pool: PgPool) {
    use scylla_auth::authz::ORGANIZATION_CREATOR_ROLE;

    let account = NewAccount::without_organization(user("solo")).with_grant(
        RoleName::new(ORGANIZATION_CREATOR_ROLE).unwrap(),
        Scope::System,
    );
    PgSignupRepository::new(pool.clone())
        .provision_user(&account.user, &account.grants)
        .await
        .expect("provision");

    PgUserRepository::new(pool.clone())
        .find_by_id(account.user.id())
        .await
        .expect("user persisted");
    let grants = PgGrantRepository::new(pool.clone())
        .list_all()
        .await
        .unwrap();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].id, account.grants[0].id);
    let organizations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM organizations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(organizations, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_grant_that_fails_rolls_back_the_account_without_an_organization(pool: PgPool) {
    let account = NewAccount::without_organization(user("solo")).with_grant(
        RoleName::new(ORGANIZATION_ADMIN_ROLE).unwrap(),
        Scope::Organization(crate::domain::ids::OrganizationId::new("missing")),
    );

    let err = PgSignupRepository::new(pool.clone())
        .provision_user(&account.user, &account.grants)
        .await
        .expect_err("a grant on a missing organization must fail");

    assert!(
        matches!(err, crate::domain::errors::DomainError::BusinessRule(_)),
        "{err:?}"
    );
    assert!(
        PgUserRepository::new(pool)
            .find_by_id(account.user.id())
            .await
            .is_err()
    );
}
