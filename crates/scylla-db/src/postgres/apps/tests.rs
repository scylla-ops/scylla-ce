use super::PgAppRepository;
use crate::domain::app::{App, AppCredential, AppKind, TRIGGER_RUNNER_APP_NAME};
use crate::domain::app::{AppName, AppSecretHash, AppSecretLabel};
use crate::domain::errors::DomainError;
use crate::domain::ids::{AppId, OrganizationId};
use crate::domain::role::RoleName;
use crate::postgres::{PgGrantRepository, PgOrganizationRepository};
use crate::test_support::prelude::*;
use scylla_auth::authz::{
    Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, ORGANIZATION_AGENT_ROLE,
    ORGANIZATION_TRIGGER_RUNNER_ROLE, PROJECT_AGENT_ROLE, Principal, SYSTEM_ADMIN_ROLE, Scope,
};
use scylla_core::application::app::AppRepository;
use sqlx::PgPool;

const TEST_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$aGFzaGhhc2g";

fn runner(org_id: &OrganizationId) -> (App, Grant) {
    let app = App::trigger_runner(org_id.clone()).unwrap();
    let grant = Grant::new(
        Principal::App(app.id().clone()),
        RoleName::new(ORGANIZATION_TRIGGER_RUNNER_ROLE).unwrap(),
        Scope::Organization(org_id.clone()),
    );
    (app, grant)
}

fn standard(org_id: &OrganizationId, name: &str) -> (App, AppCredential) {
    let app = App::create(org_id.clone(), AppName::new(name).unwrap()).unwrap();
    let credential = AppCredential::create(
        app.id().clone(),
        AppSecretLabel::new("default").unwrap(),
        AppSecretHash::new(TEST_HASH).unwrap(),
    );
    (app, credential)
}

async fn insert_app(pool: &PgPool, org_id: &OrganizationId, id: &str, name: &str, kind: &str) {
    sqlx::query("INSERT INTO apps (id, organization_id, name, kind) VALUES ($1, $2, $3, $4)")
        .bind(id)
        .bind(org_id.as_str())
        .bind(name)
        .bind(kind)
        .execute(pool)
        .await
        .unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn provision_then_find_list_and_delete(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let repo = PgAppRepository::new(pool.clone());
    let (app, grant) = runner(org.id());

    repo.provision(&app, &grant).await.expect("provision");

    let found = repo.find_by_id(app.id()).await.expect("app persisted");
    assert_eq!(found.name().as_str(), TRIGGER_RUNNER_APP_NAME);
    assert_eq!(found.kind(), AppKind::TriggerRunner);
    assert_eq!(found.organization_id(), org.id());
    assert_eq!(
        repo.find_trigger_runner(org.id()).await.unwrap().as_ref(),
        Some(app.id())
    );

    let list = repo.list_by_organization(org.id()).await.unwrap();
    assert_eq!(list.len(), 1);

    let grants = PgGrantRepository::new(pool.clone())
        .list_all()
        .await
        .unwrap();
    assert!(
        grants
            .iter()
            .any(|g| g.principal == Principal::App(app.id().clone())),
        "the runner grant must be minted with the app"
    );

    repo.delete(app.id()).await.expect("delete");
    assert!(repo.find_by_id(app.id()).await.is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_standard_app_round_trips_its_kind(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let repo = PgAppRepository::new(pool.clone());
    let (app, credential) = standard(org.id(), "ci-bot");

    repo.create_app(&app, &credential).await.unwrap();

    let found = repo.find_by_id(app.id()).await.unwrap();
    assert_eq!(found.kind(), AppKind::Standard);
    assert_eq!(repo.find_trigger_runner(org.id()).await.unwrap(), None);
}

#[sqlx::test(migrations = "../../migrations")]
async fn find_trigger_runner_ignores_a_standard_app(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let repo = PgAppRepository::new(pool.clone());
    insert_app(
        &pool,
        org.id(),
        "legacy",
        TRIGGER_RUNNER_APP_NAME,
        "standard",
    )
    .await;

    assert_eq!(repo.find_trigger_runner(org.id()).await.unwrap(), None);
    let (app, grant) = runner(org.id());
    assert!(matches!(
        repo.provision(&app, &grant).await,
        Err(DomainError::Conflict(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_second_runner_in_one_org_conflicts(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let repo = PgAppRepository::new(pool.clone());
    let (app, grant) = runner(org.id());
    repo.provision(&app, &grant).await.unwrap();

    let second = sqlx::query(
        "INSERT INTO apps (id, organization_id, name, kind) VALUES ('other', $1, 'other', 'trigger_runner')",
    )
    .bind(org.id().as_str())
    .execute(&pool)
    .await;

    let err = second.expect_err("one runner per organization");
    assert!(err.as_database_error().unwrap().is_unique_violation());
}

#[sqlx::test(migrations = "../../migrations")]
async fn duplicate_name_in_same_org_conflicts(pool: PgPool) {
    let org = seed_org(&pool, "Acme").await;
    let repo = PgAppRepository::new(pool.clone());
    let (first, first_cred) = standard(org.id(), "dup");
    repo.create_app(&first, &first_cred).await.expect("first");

    let (clash, clash_cred) = standard(org.id(), "dup");
    let err = repo
        .create_app(&clash, &clash_cred)
        .await
        .expect_err("duplicate (org, name) must fail");
    assert!(
        matches!(&err, DomainError::Conflict(m) if m == "App name already exists in this organization"),
        "{err:?}"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn cascade_org_delete_removes_apps(pool: PgPool) {
    use scylla_core::application::OrganizationRepository;

    let org = seed_org(&pool, "Acme").await;
    let repo = PgAppRepository::new(pool.clone());
    let (app, grant) = runner(org.id());
    repo.provision(&app, &grant).await.expect("provision");

    PgOrganizationRepository::new(pool.clone())
        .delete(&org)
        .await
        .expect("delete org");

    assert!(
        repo.find_by_id(app.id()).await.is_err(),
        "deleting the org cascades to its apps"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_access_list_of_an_app_names_each_of_its_grants_in_order(pool: PgPool) {
    let acme = seed_org(&pool, "Acme").await;
    let web = seed_project(&pool, &acme, "Web").await;
    let api = seed_project(&pool, &acme, "Api").await;
    let repo = PgAppRepository::new(pool.clone());
    let (app, credential) = standard(acme.id(), "ci-bot");
    repo.create_app(&app, &credential).await.unwrap();
    let (other, other_credential) = standard(acme.id(), "other");
    repo.create_app(&other, &other_credential).await.unwrap();
    let user = seed_user(&pool, "kevin").await;
    let grants = PgGrantRepository::new(pool.clone());
    let grant = |principal: &Principal, role: &str, scope: Scope| {
        Grant::new(principal.clone(), RoleName::new(role).unwrap(), scope)
    };
    let principal = Principal::App(app.id().clone());
    for g in [
        grant(
            &principal,
            PROJECT_AGENT_ROLE,
            Scope::Project(web.id().clone()),
        ),
        grant(
            &principal,
            ORGANIZATION_AGENT_ROLE,
            Scope::Organization(acme.id().clone()),
        ),
        grant(
            &principal,
            PROJECT_AGENT_ROLE,
            Scope::Project(api.id().clone()),
        ),
        grant(&principal, SYSTEM_ADMIN_ROLE, Scope::System),
        grant(
            &Principal::App(other.id().clone()),
            ORGANIZATION_AGENT_ROLE,
            Scope::Organization(acme.id().clone()),
        ),
        grant(
            &Principal::User(user.id().clone()),
            ORGANIZATION_ADMIN_ROLE,
            Scope::Organization(acme.id().clone()),
        ),
    ] {
        grants.create(&g).await.unwrap();
    }

    let access = repo.list_access(app.id()).await.unwrap();
    let unknown = repo.list_access(&AppId::new("ghost")).await.unwrap();

    let rows: Vec<(String, String, String, &str)> = access
        .iter()
        .map(|a| {
            (
                a.scope.to_string(),
                a.organization_name
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                a.project_name
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                a.role.as_str(),
            )
        })
        .collect();
    let row = |scope: String, org: &str, project: &str, role| {
        (scope, org.to_owned(), project.to_owned(), role)
    };
    assert_eq!(
        rows,
        vec![
            row("system".into(), "", "", SYSTEM_ADMIN_ROLE),
            row(
                format!("organization:{}", acme.id()),
                "Acme",
                "",
                ORGANIZATION_AGENT_ROLE
            ),
            row(
                format!("project:{}", api.id()),
                "Acme",
                "Api",
                PROJECT_AGENT_ROLE
            ),
            row(
                format!("project:{}", web.id()),
                "Acme",
                "Web",
                PROJECT_AGENT_ROLE
            ),
        ]
    );
    assert_eq!(access[0].organization_id, None);
    assert_eq!(access[3].organization_id.as_ref(), Some(acme.id()));
    assert!(unknown.is_empty());
}
