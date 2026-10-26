use super::PgProjectRepository;
use crate::domain::errors::DomainError;
use crate::postgres::PgOrganizationRepository;
use crate::test_support::prelude::*;
use scylla_auth::authz::Visibility;
use scylla_core::application::{OrganizationRepository, ProjectRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn create_then_find_round_trip(pool: PgPool) {
    let org = seed_org(&pool, "acme").await;
    let repo = PgProjectRepository::new(pool);
    let project = project(&org, "rocket");

    repo.create(&project).await.expect("create");
    let found = repo.find_by_id(project.id()).await.expect("find");

    assert_eq!(found.id(), project.id());
    assert_eq!(found.organization_id(), org.id());
    assert_eq!(found.created_at(), project.created_at());
}

#[sqlx::test(migrations = "../../migrations")]
async fn fk_violation_on_unknown_organization_is_a_business_rule(pool: PgPool) {
    let phantom = org("never-persisted");
    let project = project(&phantom, "orphan");
    let repo = PgProjectRepository::new(pool);

    assert!(matches!(
        repo.create(&project).await,
        Err(DomainError::BusinessRule(_)),
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn list_by_organization_filters_other_orgs(pool: PgPool) {
    let org_a = seed_org(&pool, "org-a").await;
    let org_b = seed_org(&pool, "org-b").await;
    let repo = PgProjectRepository::new(pool);

    repo.create(&project(&org_a, "a1")).await.unwrap();
    repo.create(&project(&org_a, "a2")).await.unwrap();
    repo.create(&project(&org_b, "b1")).await.unwrap();

    assert_eq!(
        repo.list_by_organization(org_a.id(), None, &Visibility::All)
            .await
            .unwrap()
            .metadata()
            .total_count(),
        2,
    );
    assert_eq!(
        repo.list_by_organization(org_b.id(), None, &Visibility::All)
            .await
            .unwrap()
            .metadata()
            .total_count(),
        1,
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn cascade_organization_delete_removes_projects(pool: PgPool) {
    let org = seed_org(&pool, "doomed").await;
    let project_repo = PgProjectRepository::new(pool.clone());
    let project = project(&org, "child");
    project_repo.create(&project).await.unwrap();

    PgOrganizationRepository::new(pool)
        .delete(&org)
        .await
        .unwrap();

    assert!(matches!(
        project_repo.find_by_id(project.id()).await,
        Err(DomainError::NotFound(_)),
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn provision_with_owner_writes_the_owner_grant(pool: PgPool) {
    use crate::domain::role::RoleName;
    use crate::postgres::PgGrantRepository;
    use scylla_auth::authz::{Grant, GrantRepository, PROJECT_ADMIN_ROLE, Principal, Scope};

    let org = seed_org(&pool, "acme").await;
    let owner = seed_user(&pool, "alice").await;
    let project = project(&org, "rocket");
    let grant = Grant::new(
        Principal::User(owner.id().clone()),
        RoleName::new(PROJECT_ADMIN_ROLE).unwrap(),
        Scope::Project(project.id().clone()),
    );

    let repo = PgProjectRepository::new(pool.clone());
    repo.provision_with_owner(&project, &grant)
        .await
        .expect("provision");

    assert_eq!(
        repo.find_by_id(project.id()).await.unwrap().id(),
        project.id()
    );
    let grants = PgGrantRepository::new(pool).list_all().await.unwrap();
    assert!(
        grants.iter().any(|g| {
            matches!(&g.principal, Principal::User(u) if u.as_str() == owner.id().as_str())
                && g.role.as_str() == PROJECT_ADMIN_ROLE
                && matches!(&g.scope, Scope::Project(p) if p.as_str() == project.id().as_str())
        }),
        "creator should hold a project-admin owner grant",
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn provision_with_owner_rolls_back_on_failure(pool: PgPool) {
    use crate::domain::role::RoleName;
    use crate::postgres::PgGrantRepository;
    use scylla_auth::authz::{Grant, GrantRepository, PROJECT_ADMIN_ROLE, Principal, Scope};

    let org = seed_org(&pool, "acme").await;
    let owner = seed_user(&pool, "alice").await;
    let project = project(&org, "rocket");
    let repo = PgProjectRepository::new(pool.clone());

    repo.create(&project).await.expect("seed the clash");

    let grant = Grant::new(
        Principal::User(owner.id().clone()),
        RoleName::new(PROJECT_ADMIN_ROLE).unwrap(),
        Scope::Project(project.id().clone()),
    );
    assert!(
        repo.provision_with_owner(&project, &grant).await.is_err(),
        "a duplicate project id must fail the transaction"
    );

    let grants = PgGrantRepository::new(pool).list_all().await.unwrap();
    assert!(
        grants.is_empty(),
        "the owner grant is rolled back with the project insert"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_project_listing_shows_only_what_the_caller_holds(pool: PgPool) {
    use crate::domain::caller::CallerContext;
    use crate::domain::role::RoleName;
    use crate::postgres::{PgAuthzEntityProvider, PgGrantRepository, PgRoleRepository};
    use scylla_auth::audit::NoopAuditLog;
    use scylla_auth::authz::{
        Grant, GrantRepository, PROJECT_VIEWER_ROLE, Principal, Scope, Visibility,
        VisibilityResolver,
    };
    use scylla_auth::cedar::CedarPermissionService;
    use std::sync::Arc;

    let org = seed_org(&pool, "acme").await;
    let mine = seed_project(&pool, &org, "apollo").await;
    let _theirs = seed_project(&pool, &org, "zeus").await;
    let alice = seed_user(&pool, "alice").await;

    PgGrantRepository::new(pool.clone())
        .create(&Grant::new(
            Principal::User(alice.id().clone()),
            RoleName::new(PROJECT_VIEWER_ROLE).unwrap(),
            Scope::Project(mine.id().clone()),
        ))
        .await
        .unwrap();

    let permission = CedarPermissionService::new(
        Arc::new(PgAuthzEntityProvider::new(pool.clone())),
        Arc::new(PgRoleRepository::new(pool.clone())),
        Arc::new(PgGrantRepository::new(pool.clone())),
        Arc::new(NoopAuditLog),
    )
    .await
    .expect("cedar");

    let visible = permission
        .visible_scopes(&CallerContext::User(alice.id().clone()), "readProject")
        .await
        .expect("visible scopes");
    assert_eq!(
        visible,
        Visibility::Scoped {
            orgs: vec![],
            projects: vec![mine.id().clone()],
        }
    );

    let page = PgProjectRepository::new(pool)
        .list_by_organization(org.id(), None, &visible)
        .await
        .expect("list");
    assert_eq!(
        page.metadata().total_count(),
        1,
        "the other project is not hers"
    );
    assert_eq!(page.items()[0].id(), mine.id());
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_update_from_a_stale_read_is_stale_and_the_first_write_wins(pool: PgPool) {
    use crate::domain::project::ProjectName;

    let org = seed_org(&pool, "acme").await;
    let repo = PgProjectRepository::new(pool);
    let project = project(&org, "one");
    repo.create(&project).await.expect("create");

    let mut first = repo.find_by_id(project.id()).await.expect("first read");
    let mut second = repo.find_by_id(project.id()).await.expect("second read");
    assert_eq!(first.version(), 0);

    first.update_name(ProjectName::new("a").unwrap()).unwrap();
    let written = repo.update(&first).await.expect("first write");
    assert_eq!(written.version(), 1);

    second.update_name(ProjectName::new("b").unwrap()).unwrap();
    let err = repo.update(&second).await.expect_err("stale write");
    assert!(matches!(err, DomainError::Stale(_)));

    let stored = repo.find_by_id(project.id()).await.expect("find");
    assert_eq!(stored.name().as_str(), "a");
    assert_eq!(stored.version(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_delete_with_a_stale_version_is_stale(pool: PgPool) {
    use crate::domain::project::ProjectName;

    let org = seed_org(&pool, "acme").await;
    let repo = PgProjectRepository::new(pool);
    let project = project(&org, "one");
    repo.create(&project).await.expect("create");

    let stale = repo.find_by_id(project.id()).await.expect("read");
    let mut fresh = stale.clone();
    fresh
        .update_name(ProjectName::new("renamed").unwrap())
        .unwrap();
    let fresh = repo.update(&fresh).await.expect("write");

    let err = repo.delete(&stale).await.expect_err("stale delete");
    assert!(matches!(err, DomainError::Stale(_)));
    assert!(repo.find_by_id(project.id()).await.is_ok());

    repo.delete(&fresh)
        .await
        .expect("delete at the current version");
    assert!(matches!(
        repo.find_by_id(project.id()).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_write_on_a_missing_row_is_not_found(pool: PgPool) {
    let org = seed_org(&pool, "acme").await;
    let repo = PgProjectRepository::new(pool);
    let never_persisted = project(&org, "ghost");

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
async fn the_projects_of_a_user_are_the_ones_its_grants_let_it_read(pool: PgPool) {
    use crate::domain::role::RoleName;
    use crate::postgres::PgGrantRepository;
    use scylla_auth::authz::{
        Grant, GrantRepository, ORGANIZATION_MEMBER_ROLE, ORGANIZATION_VIEWER_ROLE,
        PROJECT_VIEWER_ROLE, Principal, Scope,
    };

    let org = seed_org(&pool, "acme").await;
    let apollo = seed_project(&pool, &org, "apollo").await;
    let zeus = seed_project(&pool, &org, "zeus").await;
    let grants = PgGrantRepository::new(pool.clone());
    let repo = PgProjectRepository::new(pool.clone());
    let holding = async |name: &str, role: &str, scope: Scope| {
        let user = seed_user(&pool, name).await;
        grants
            .create(&Grant::new(
                Principal::User(user.id().clone()),
                RoleName::new(role).unwrap(),
                scope,
            ))
            .await
            .unwrap();
        let page = repo
            .list_for_user(user.id(), "readProject", None)
            .await
            .unwrap();
        assert_eq!(page.metadata().total_count(), page.items().len() as u64);
        let mut ids: Vec<_> = page.items().iter().map(|p| p.id().clone()).collect();
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        ids
    };

    let member = holding(
        "member",
        ORGANIZATION_MEMBER_ROLE,
        Scope::Organization(org.id().clone()),
    )
    .await;
    assert!(
        member.is_empty(),
        "a member sees that the organization exists"
    );

    let viewer = holding(
        "viewer",
        ORGANIZATION_VIEWER_ROLE,
        Scope::Organization(org.id().clone()),
    )
    .await;
    let mut every = vec![apollo.id().clone(), zeus.id().clone()];
    every.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    assert_eq!(viewer, every);

    let reader = holding(
        "reader",
        PROJECT_VIEWER_ROLE,
        Scope::Project(apollo.id().clone()),
    )
    .await;
    assert_eq!(reader, vec![apollo.id().clone()]);
}
