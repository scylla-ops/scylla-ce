use super::PgSecretRepository;
use crate::domain::errors::DomainError;
use crate::domain::secret::{Secret, SecretChange, SecretDescription, SecretName};
use crate::test_support::prelude::*;
use scylla_core::application::secret::SecretRepository;
use sqlx::PgPool;

fn secret(project: &crate::domain::project::Project) -> Secret {
    Secret::create(
        project.id().clone(),
        SecretName::new("DB_PASSWORD").unwrap(),
        SecretDescription::new("old").unwrap(),
        vec![0xAA],
    )
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_update_writes_only_the_fields_it_carries_and_keeps_the_rest(pool: PgPool) {
    let (_, project, _) = seed_org_project_pipeline(&pool, "secret").await;
    let repo = PgSecretRepository::new(pool);
    let created = secret(&project);
    repo.create(&created).await.unwrap();

    let change = SecretChange::new(created.id().clone(), None, Some(vec![0xBB, 0xCC]));
    let returned = repo.update(&change).await.unwrap();

    let stored = repo.find_by_id(created.id()).await.unwrap();
    assert_eq!(returned, stored);
    assert_eq!(stored.encrypted_value(), &[0xBB, 0xCC]);
    assert_eq!(stored.description().as_str(), "old");
    assert_eq!(stored.name(), created.name());
    assert_eq!(stored.created_at(), created.created_at());
    assert_eq!(stored.updated_at(), change.updated_at());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_description_edit_from_an_old_read_keeps_a_newer_value(pool: PgPool) {
    let (_, project, _) = seed_org_project_pipeline(&pool, "race").await;
    let repo = PgSecretRepository::new(pool);
    let created = secret(&project);
    repo.create(&created).await.unwrap();
    let old_read = repo.find_by_id(created.id()).await.unwrap();

    repo.update(&SecretChange::new(
        created.id().clone(),
        None,
        Some(vec![0xBB, 0xCC]),
    ))
    .await
    .unwrap();
    repo.update(&SecretChange::new(
        old_read.id().clone(),
        Some(SecretDescription::new("new").unwrap()),
        None,
    ))
    .await
    .unwrap();

    let stored = repo.find_by_id(created.id()).await.unwrap();
    assert_eq!(stored.encrypted_value(), &[0xBB, 0xCC]);
    assert_eq!(stored.description().as_str(), "new");
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_update_of_a_deleted_secret_is_not_found(pool: PgPool) {
    let (_, project, _) = seed_org_project_pipeline(&pool, "gone").await;
    let repo = PgSecretRepository::new(pool);
    let never_stored = secret(&project);

    assert!(matches!(
        repo.update(&SecretChange::new(
            never_stored.id().clone(),
            None,
            Some(vec![0xBB])
        ))
        .await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_migration_gives_update_to_each_role_that_deletes_secrets(pool: PgPool) {
    sqlx::raw_sql(
        "INSERT INTO roles (id, key, name, scope_kind) VALUES
             ('keeper', 'keeper', 'Keeper', 'project'),
             ('reader', 'reader', 'Reader', 'project');
         INSERT INTO role_permissions (role_id, permission) VALUES
             ('keeper', 'deleteSecret'), ('reader', 'listSecrets');",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(include_str!(
        "../../../../../migrations/20261025120000_update_secret_permission.sql"
    ))
    .execute(&pool)
    .await
    .unwrap();

    let holders: Vec<String> = sqlx::query_scalar(
        "SELECT role_id FROM role_permissions WHERE permission = 'updateSecret' ORDER BY role_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(holders, ["keeper"]);
}
