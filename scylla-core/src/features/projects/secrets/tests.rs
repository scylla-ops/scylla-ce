//! Unit tests of the secrets service: the opening of a value, which no RPC shows.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::{Email, NewUser};
use scylla_domain::projects::secrets::{NewSecret, SecretId, SecretName, SecretValue};
use scylla_domain::projects::{NewProject, ProjectDescription, ProjectId, ProjectName};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Name;
use scylla_extension::Hooks;
use sqlx::PgPool;

use super::service;
use crate::features::identity::accounts;
use crate::features::projects::service as projects;
use crate::infra::secret::Sealer;
use crate::Error;

/// Creates a user and a project in its personal workspace.
async fn project(pool: &PgPool) -> (Actor, ProjectId) {
    let new = NewUser {
        email: Email::parse(format!("{}@test.example", ActorId::generate())).unwrap(),
        email_verified: true,
        display_name: Name::parse("Test".to_owned()).unwrap(),
        handle: None,
        password: None,
    };
    let author = accounts::create_user(pool, new).await.unwrap();
    let workspace_id = sqlx::query_scalar!(
        "SELECT workspace_id FROM workspace_owners WHERE user_id = $1",
        author.id.to_db()
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let new = NewProject {
        workspace_id: WorkspaceId::from_db(workspace_id),
        created_by: author.id,
        name: ProjectName::parse("alpha".to_owned()).unwrap(),
        description: ProjectDescription::default(),
    };
    let project = projects::create(pool, &Hooks::default(), author.clone(), new)
        .await
        .unwrap();
    (author, project.id)
}

/// Creates a secret with a name and a value, and returns its id.
async fn secret(
    pool: &PgPool,
    sealer: &Sealer,
    author: &Actor,
    project: ProjectId,
    name: &str,
    value: &str,
) -> SecretId {
    let new = NewSecret {
        project_id: project,
        created_by: author.id,
        name: name_of(name),
        value: SecretValue::parse(value.to_owned()).unwrap(),
    };
    service::create(pool, Some(sealer), new).await.unwrap().id
}

/// Makes a name.
fn name_of(name: &str) -> SecretName {
    SecretName::parse(name.to_owned()).unwrap()
}

/// A value opens with the id of its secret. A sealed value copied to another secret does not
/// open, and the server needs its key.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn reveal_opens_a_value_only_for_its_secret(pool: PgPool) {
    // given
    let (sealer, _) = Sealer::generate();
    let (author, project) = project(&pool).await;
    let first = secret(&pool, &sealer, &author, project, "FIRST", "first value").await;
    let second = secret(&pool, &sealer, &author, project, "SECOND", "second value").await;
    sqlx::query!(
        "UPDATE secrets SET sealed_value = (SELECT sealed_value FROM secrets WHERE id = $1)
         WHERE id = $2",
        first.to_db(),
        second.to_db(),
    )
    .execute(&pool)
    .await
    .unwrap();

    // when
    let opened = service::reveal(&pool, Some(&sealer), project, &name_of("FIRST")).await;
    let copied = service::reveal(&pool, Some(&sealer), project, &name_of("SECOND")).await;
    let unknown = service::reveal(&pool, Some(&sealer), project, &name_of("OTHER")).await;
    let no_key = service::reveal(&pool, None, project, &name_of("FIRST")).await;

    // then
    assert_eq!(opened.unwrap().expose(), "first value");
    assert!(matches!(copied, Err(Error::Internal(_))), "{copied:?}");
    assert!(
        matches!(unknown, Err(Error::NotFound("secret"))),
        "{unknown:?}"
    );
    let reason = match no_key {
        Err(Error::Rejected(rejection)) => rejection.code,
        other => panic!("{other:?}"),
    };
    assert_eq!(reason, "SECRET_KEY_MISSING");
}

/// The database keeps the value sealed, never in clear.
#[sqlx::test(migrator = "crate::infra::db::MIGRATOR")]
async fn create_stores_no_value_in_clear(pool: PgPool) {
    // given
    let (sealer, _) = Sealer::generate();
    let (author, project) = project(&pool).await;

    // when
    let id = secret(&pool, &sealer, &author, project, "TOKEN", "plain-marker").await;

    // then
    let sealed = sqlx::query_scalar!("SELECT sealed_value FROM secrets WHERE id = $1", id.to_db())
        .fetch_one(&pool)
        .await
        .unwrap();
    let marker = b"plain-marker";
    assert!(!sealed.windows(marker.len()).any(|window| window == marker));
}
