use super::PgOrganizationRepository;
use crate::domain::errors::DomainError;
use crate::domain::organization::{OrganizationDescription, OrganizationName};
use crate::test_support::prelude::*;
use scylla_core::application::OrganizationRepository;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn round_trip_with_none_description(pool: PgPool) {
    let repo = PgOrganizationRepository::new(pool);
    let org = org("Acme");

    repo.create(&org).await.expect("create");
    let found = repo.find_by_id(org.id()).await.expect("find");

    assert_eq!(found.id(), org.id());
    assert!(found.description().is_none());
    assert_eq!(found.created_at(), org.created_at());
}

#[sqlx::test(migrations = "../../migrations")]
async fn round_trip_with_some_description(pool: PgPool) {
    let repo = PgOrganizationRepository::new(pool);
    let org = OrgBuilder::new("Globex")
        .description("Worldwide subsidiary")
        .build();
    repo.create(&org).await.expect("create");

    let found = repo.find_by_id(org.id()).await.expect("find");
    assert_eq!(
        found.description().map(OrganizationDescription::as_str),
        Some("Worldwide subsidiary"),
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_all_includes_inactive(pool: PgPool) {
    let repo = PgOrganizationRepository::new(pool);
    repo.create(&org("Active1")).await.expect("seed");
    repo.create(&org("Active2")).await.expect("seed");
    repo.create(&OrgBuilder::new("Dormant").is_active(false).build())
        .await
        .expect("seed inactive");

    let all = repo.list_all(None).await.expect("list");
    assert_eq!(all.metadata().total_count(), 3);
}

#[sqlx::test(migrations = "../../migrations")]
async fn revoke_all_access_strips_the_whole_org_subtree(pool: PgPool) {
    use crate::domain::role::RoleName;
    use crate::postgres::PgGrantRepository;
    use scylla_auth::authz::{
        Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, PROJECT_ADMIN_ROLE,
        PROJECT_DEVELOPER_ROLE, Principal, SYSTEM_ADMIN_ROLE, Scope,
    };

    let org = seed_org(&pool, "acme").await;
    let other_org = seed_org(&pool, "globex").await;
    let project = seed_project(&pool, &org, "apollo").await;
    let other_project = seed_project(&pool, &other_org, "zeus").await;
    let victim = seed_user(&pool, "victim").await;
    let colleague = seed_user(&pool, "colleague").await;

    let grants = PgGrantRepository::new(pool.clone());
    let role = |name: &str| RoleName::new(name).unwrap();
    let victim_principal = Principal::User(victim.id().clone());

    let doomed = [
        Grant::new(
            victim_principal.clone(),
            role(ORGANIZATION_ADMIN_ROLE),
            Scope::Organization(org.id().clone()),
        ),
        Grant::new(
            victim_principal.clone(),
            role(PROJECT_ADMIN_ROLE),
            Scope::Project(project.id().clone()),
        ),
        Grant::new(
            victim_principal.clone(),
            role(PROJECT_DEVELOPER_ROLE),
            Scope::Project(project.id().clone()),
        ),
    ];
    let survivors = [
        Grant::new(
            victim_principal.clone(),
            role(PROJECT_ADMIN_ROLE),
            Scope::Project(other_project.id().clone()),
        ),
        Grant::new(
            victim_principal.clone(),
            role(SYSTEM_ADMIN_ROLE),
            Scope::System,
        ),
        Grant::new(
            Principal::User(colleague.id().clone()),
            role(ORGANIZATION_ADMIN_ROLE),
            Scope::Organization(org.id().clone()),
        ),
    ];
    for g in doomed.iter().chain(survivors.iter()) {
        grants.create(g).await.unwrap();
    }

    let removed = grants
        .revoke_all(&victim_principal, &Scope::Organization(org.id().clone()))
        .await
        .expect("revoke all");
    assert_eq!(removed, 3, "the org grant plus both project grants");

    let remaining = grants.list_all().await.unwrap();
    for g in &doomed {
        assert!(
            !remaining.iter().any(|r| r.id == g.id),
            "grant {} should have gone with the organization access",
            g.id,
        );
    }
    for g in &survivors {
        assert!(
            remaining.iter().any(|r| r.id == g.id),
            "grant {} is outside the revoked scope and must survive",
            g.id,
        );
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_changes_description_to_none(pool: PgPool) {
    let repo = PgOrganizationRepository::new(pool);
    let mut org = OrgBuilder::new("Pied Piper")
        .description("starts with one")
        .build();
    repo.create(&org).await.expect("create");

    org.update_description(None).unwrap();
    repo.update(&org).await.expect("update");

    assert!(
        repo.find_by_id(org.id())
            .await
            .unwrap()
            .description()
            .is_none()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_update_from_a_stale_read_is_stale_and_the_first_write_wins(pool: PgPool) {
    let org = seed_org(&pool, "acme").await;
    let repo = PgOrganizationRepository::new(pool);

    let mut first = repo.find_by_id(org.id()).await.expect("first read");
    let mut second = repo.find_by_id(org.id()).await.expect("second read");
    assert_eq!(first.version(), 0);

    first
        .update_name(OrganizationName::new("a").unwrap())
        .unwrap();
    let written = repo.update(&first).await.expect("first write");
    assert_eq!(written.version(), 1);

    second.set_active(false);
    let err = repo.update(&second).await.expect_err("stale write");
    assert!(matches!(err, DomainError::Stale(_)));

    let stored = repo.find_by_id(org.id()).await.expect("find");
    assert_eq!(stored.name().as_str(), "a");
    assert!(stored.is_active());
    assert_eq!(stored.version(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_delete_with_a_stale_version_is_stale(pool: PgPool) {
    let org = seed_org(&pool, "acme").await;
    let repo = PgOrganizationRepository::new(pool);

    let stale = repo.find_by_id(org.id()).await.expect("read");
    let mut fresh = stale.clone();
    fresh
        .update_name(OrganizationName::new("renamed").unwrap())
        .unwrap();
    let fresh = repo.update(&fresh).await.expect("write");

    let err = repo.delete(&stale).await.expect_err("stale delete");
    assert!(matches!(err, DomainError::Stale(_)));
    assert!(repo.find_by_id(org.id()).await.is_ok());

    repo.delete(&fresh)
        .await
        .expect("delete at the current version");
    assert!(matches!(
        repo.find_by_id(org.id()).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_write_on_a_missing_row_is_not_found(pool: PgPool) {
    let repo = PgOrganizationRepository::new(pool);
    let never_persisted = org("ghost");

    assert!(matches!(
        repo.update(&never_persisted).await,
        Err(DomainError::NotFound(_))
    ));
    assert!(matches!(
        repo.delete(&never_persisted).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_members_are_the_users_with_a_grant_on_the_organization_or_on_its_projects(
    pool: PgPool,
) {
    use crate::domain::app::{App, AppName};
    use crate::domain::role::RoleName;
    use crate::postgres::{PgAppRepository, PgGrantRepository};
    use scylla_auth::authz::{
        Grant, GrantRepository, ORGANIZATION_AGENT_ROLE, ORGANIZATION_MEMBER_ROLE,
        PROJECT_DEVELOPER_ROLE, Principal, SYSTEM_ADMIN_ROLE, Scope,
    };
    use scylla_core::application::AppRepository;

    let org = seed_org(&pool, "acme").await;
    let other_org = seed_org(&pool, "globex").await;
    let project = seed_project(&pool, &org, "apollo").await;
    let member = seed_user(&pool, "member").await;
    let developer = seed_user(&pool, "developer").await;
    let outsider = seed_user(&pool, "outsider").await;
    let operator = seed_user(&pool, "operator").await;

    let grants = PgGrantRepository::new(pool.clone());
    let role = |name: &str| RoleName::new(name).unwrap();
    for grant in [
        Grant::new(
            Principal::User(member.id().clone()),
            role(ORGANIZATION_MEMBER_ROLE),
            Scope::Organization(org.id().clone()),
        ),
        Grant::new(
            Principal::User(developer.id().clone()),
            role(PROJECT_DEVELOPER_ROLE),
            Scope::Project(project.id().clone()),
        ),
        Grant::new(
            Principal::User(outsider.id().clone()),
            role(ORGANIZATION_MEMBER_ROLE),
            Scope::Organization(other_org.id().clone()),
        ),
        Grant::new(
            Principal::User(operator.id().clone()),
            role(SYSTEM_ADMIN_ROLE),
            Scope::System,
        ),
    ] {
        grants.create(&grant).await.unwrap();
    }
    let agent = App::create(org.id().clone(), AppName::new("agent").unwrap()).unwrap();
    PgAppRepository::new(pool.clone())
        .provision(
            &agent,
            &Grant::new(
                Principal::App(agent.id().clone()),
                role(ORGANIZATION_AGENT_ROLE),
                Scope::Organization(org.id().clone()),
            ),
        )
        .await
        .unwrap();

    let repo = PgOrganizationRepository::new(pool);
    assert_eq!(repo.count_members(org.id()).await.unwrap(), 2);
    for (user, expected) in [
        (&member, true),
        (&developer, true),
        (&outsider, false),
        (&operator, false),
    ] {
        assert_eq!(
            repo.has_member(org.id(), user.id()).await.unwrap(),
            expected,
            "{}",
            user.username().as_str()
        );
    }
    let agent_as_user = crate::domain::ids::UserId::new(agent.id().as_str());
    assert!(!repo.has_member(org.id(), &agent_as_user).await.unwrap());
}
