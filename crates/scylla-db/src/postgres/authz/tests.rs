use super::PgAuthzEntityProvider;
use crate::domain::app::{App, AppCredential, AppName, AppSecretHash, AppSecretLabel};
use crate::domain::ids::{AppCredentialId, GrantId, SecretId, TriggerId};
use crate::domain::permission::ResourceRef;
use crate::domain::role::RoleName;
use crate::domain::secret::{Secret, SecretDescription, SecretName};
use crate::domain::trigger::{CronSpec, Trigger, TriggerName, TriggerSource};
use crate::postgres::{
    PgAppRepository, PgGrantRepository, PgSecretRepository, PgTriggerRepository,
};
use crate::test_support::prelude::*;
use scylla_auth::authz::{
    AuthzEntityProvider, Grant, GrantRepository, ORGANIZATION_VIEWER_ROLE, PROJECT_VIEWER_ROLE,
    Principal, ResourceAncestors, SYSTEM_ADMIN_ROLE, Scope,
};
use scylla_core::application::TriggerRepository;
use scylla_core::application::app::AppRepository;
use scylla_core::application::secret::SecretRepository;
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn a_job_resolves_to_its_pipeline_project_and_organization(pool: PgPool) {
    let (org, project, pipeline) = seed_org_project_pipeline(&pool, "job").await;
    let job = seed_job(&pool, &pipeline).await;

    let ancestors = PgAuthzEntityProvider::new(pool)
        .resource_ancestors(&ResourceRef::Job(job.id().clone()))
        .await
        .unwrap();

    assert_eq!(ancestors.organization.as_ref(), Some(org.id()));
    assert_eq!(ancestors.project.as_ref(), Some(project.id()));
    assert_eq!(ancestors.pipeline.as_ref(), Some(pipeline.id()));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_secret_resolves_to_its_project_and_organization(pool: PgPool) {
    let (org, project, _) = seed_org_project_pipeline(&pool, "secret").await;
    let secret = Secret::create(
        project.id().clone(),
        SecretName::new("DB_PASSWORD").unwrap(),
        SecretDescription::new("").unwrap(),
        vec![0xAA],
    );
    PgSecretRepository::new(pool.clone())
        .create(&secret)
        .await
        .unwrap();

    let ancestors = PgAuthzEntityProvider::new(pool)
        .resource_ancestors(&ResourceRef::Secret(secret.id().clone()))
        .await
        .unwrap();

    assert_eq!(ancestors.organization.as_ref(), Some(org.id()));
    assert_eq!(ancestors.project.as_ref(), Some(project.id()));
    assert!(ancestors.pipeline.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_secret_has_no_ancestors(pool: PgPool) {
    let ancestors = PgAuthzEntityProvider::new(pool)
        .resource_ancestors(&ResourceRef::Secret(SecretId::new("missing")))
        .await
        .unwrap();

    assert!(ancestors.organization.is_none());
    assert!(ancestors.project.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_trigger_resolves_to_its_pipeline_project_and_organization(pool: PgPool) {
    let (org, project, pipeline) = seed_org_project_pipeline(&pool, "trigger").await;
    let trigger = Trigger::create(
        pipeline.id().clone(),
        TriggerName::new("nightly").unwrap(),
        TriggerSource::Cron(CronSpec::new("0 9 * * *").unwrap()),
        vec![],
    )
    .unwrap();
    PgTriggerRepository::new(pool.clone())
        .create(&trigger, None)
        .await
        .unwrap();

    let ancestors = PgAuthzEntityProvider::new(pool)
        .resource_ancestors(&ResourceRef::Trigger(trigger.id().clone()))
        .await
        .unwrap();

    assert_eq!(ancestors.organization.as_ref(), Some(org.id()));
    assert_eq!(ancestors.project.as_ref(), Some(project.id()));
    assert_eq!(ancestors.pipeline.as_ref(), Some(pipeline.id()));
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_trigger_has_no_ancestors(pool: PgPool) {
    let ancestors = PgAuthzEntityProvider::new(pool)
        .resource_ancestors(&ResourceRef::Trigger(TriggerId::new("missing")))
        .await
        .unwrap();

    assert!(ancestors.organization.is_none());
    assert!(ancestors.project.is_none());
    assert!(ancestors.pipeline.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_app_secret_resolves_to_its_app_and_organization(pool: PgPool) {
    let org = seed_org(&pool, "app-secret").await;
    let app = App::create(org.id().clone(), AppName::new("ci-bot").unwrap()).unwrap();
    let credential = AppCredential::create(
        app.id().clone(),
        AppSecretLabel::new("default").unwrap(),
        AppSecretHash::new("$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$aGFzaGhhc2g").unwrap(),
    );
    PgAppRepository::new(pool.clone())
        .create_app(&app, &credential)
        .await
        .unwrap();

    let ancestors = PgAuthzEntityProvider::new(pool)
        .resource_ancestors(&ResourceRef::AppSecret(credential.id().clone()))
        .await
        .unwrap();

    assert_eq!(ancestors.organization.as_ref(), Some(org.id()));
    assert_eq!(ancestors.app.as_ref(), Some(app.id()));
    assert!(ancestors.project.is_none());
    assert!(ancestors.pipeline.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_app_secret_has_no_ancestors(pool: PgPool) {
    let ancestors = PgAuthzEntityProvider::new(pool)
        .resource_ancestors(&ResourceRef::AppSecret(AppCredentialId::new("missing")))
        .await
        .unwrap();

    assert!(ancestors.organization.is_none());
    assert!(ancestors.app.is_none());
}

async fn grant_ancestors(pool: &PgPool, role: &str, scope: Scope) -> ResourceAncestors {
    let user = seed_user(pool, "grantee").await;
    let grant = Grant::new(
        Principal::User(user.id().clone()),
        RoleName::new(role).unwrap(),
        scope,
    );
    PgGrantRepository::new(pool.clone())
        .create(&grant)
        .await
        .unwrap();
    PgAuthzEntityProvider::new(pool.clone())
        .resource_ancestors(&ResourceRef::Grant(GrantId::new(grant.id)))
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_project_grant_resolves_to_its_project_and_organization(pool: PgPool) {
    let (org, project, _) = seed_org_project_pipeline(&pool, "grant").await;

    let ancestors = grant_ancestors(
        &pool,
        PROJECT_VIEWER_ROLE,
        Scope::Project(project.id().clone()),
    )
    .await;

    assert_eq!(ancestors.organization.as_ref(), Some(org.id()));
    assert_eq!(ancestors.project.as_ref(), Some(project.id()));
    assert!(ancestors.pipeline.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_organization_grant_resolves_to_its_organization(pool: PgPool) {
    let org = seed_org(&pool, "grant").await;

    let ancestors = grant_ancestors(
        &pool,
        ORGANIZATION_VIEWER_ROLE,
        Scope::Organization(org.id().clone()),
    )
    .await;

    assert_eq!(ancestors.organization.as_ref(), Some(org.id()));
    assert!(ancestors.project.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_system_grant_has_no_ancestors(pool: PgPool) {
    let ancestors = grant_ancestors(&pool, SYSTEM_ADMIN_ROLE, Scope::System).await;

    assert!(ancestors.organization.is_none());
    assert!(ancestors.project.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_grant_has_no_ancestors(pool: PgPool) {
    let ancestors = PgAuthzEntityProvider::new(pool)
        .resource_ancestors(&ResourceRef::Grant(GrantId::new("missing")))
        .await
        .unwrap();

    assert!(ancestors.organization.is_none());
    assert!(ancestors.project.is_none());
}

#[sqlx::test(migrations = "../../migrations")]
async fn every_change_to_grants_and_roles_bumps_the_policy_version(pool: PgPool) {
    let provider = PgAuthzEntityProvider::new(pool.clone());
    let org = seed_org(&pool, "versioned").await;
    let user = seed_user(&pool, "versioned").await;
    let start = provider.policy_version().await.unwrap();

    PgGrantRepository::new(pool.clone())
        .create(&Grant::new(
            Principal::User(user.id().clone()),
            RoleName::new(ORGANIZATION_VIEWER_ROLE).unwrap(),
            Scope::Organization(org.id().clone()),
        ))
        .await
        .unwrap();
    let granted = provider.policy_version().await.unwrap();
    assert!(granted > start);

    sqlx::query("DELETE FROM role_permissions WHERE role_id = $1 AND permission = 'readProject'")
        .bind(ORGANIZATION_VIEWER_ROLE)
        .execute(&pool)
        .await
        .unwrap();
    let edited = provider.policy_version().await.unwrap();
    assert!(edited > granted);

    sqlx::query("DELETE FROM organizations WHERE id = $1")
        .bind(org.id().as_str())
        .execute(&pool)
        .await
        .unwrap();
    let cascaded = provider.policy_version().await.unwrap();
    assert!(cascaded > edited);
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM grants WHERE scope_id = $1")
        .bind(org.id().as_str())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(left, 0);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_role_sits_under_its_owner_and_a_platform_role_under_nothing(pool: PgPool) {
    use scylla_auth::authz::{Role, RoleKind, RoleRepository, ScopeKind};
    let org = seed_org(&pool, "owner").await;
    let role = Role::new_custom(
        crate::domain::role::RoleDisplayName::new("Deployer").unwrap(),
        crate::domain::role::RoleDescription::new("").unwrap(),
        ScopeKind::Project,
        RoleKind::Member,
        Some(org.id().clone()),
        vec!["runPipeline".into()],
    );
    crate::postgres::PgRoleRepository::new(pool.clone())
        .create(&role)
        .await
        .unwrap();
    let provider = PgAuthzEntityProvider::new(pool);

    let owned = provider
        .resource_ancestors(&ResourceRef::Role(RoleName::new(&role.id).unwrap()))
        .await
        .unwrap();
    assert_eq!(owned.organization.as_ref(), Some(org.id()));
    for platform in [SYSTEM_ADMIN_ROLE, "ghost"] {
        let ancestors = provider
            .resource_ancestors(&ResourceRef::Role(RoleName::new(platform).unwrap()))
            .await
            .unwrap();
        assert!(ancestors.organization.is_none(), "{platform}");
    }
}
