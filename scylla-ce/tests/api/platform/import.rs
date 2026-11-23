//! The tests of the one-off import of the old product, against its real schema.

use argon2::{Argon2, PasswordHasher};
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_testkit::Legacy;
use sqlx::postgres::PgPool;

use crate::app;

/// The password of the old users.
const PASSWORD: &str = "correct horse battery";

/// Makes a database of the old product next to the test database, with some rows and a real
/// hash of `PASSWORD` for each user.
async fn legacy(pool: &PgPool) -> Legacy {
    let legacy = Legacy::create(pool).await;
    let hash = Argon2::default()
        .hash_password(PASSWORD.as_bytes())
        .unwrap()
        .to_string();
    sqlx::query("UPDATE users SET password_hash = $1")
        .bind(hash)
        .execute(&legacy.pool)
        .await
        .unwrap();
    legacy
}

/// The id in this base of an old ULID of the test.
fn id_of(ulid: &str) -> uuid::Uuid {
    let alphabet = "0123456789abcdefghjkmnpqrstvwxyz";
    let bits = ulid.chars().fold(0u128, |bits, c| {
        (bits << 5) | alphabet.find(c).unwrap() as u128
    });
    uuid::Uuid::from_u128(bits)
}

/// The users keep their passwords and their state, the organizations become workspaces with
/// their owners and roles, the projects keep their ids and roles, the secrets open with the new
/// key, and a row that does not fit is named in the report. A second import is refused.
#[sqlx::test(migrations = false)]
async fn import_moves_the_old_product(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let legacy = legacy(&pool).await;

    // when
    let report = legacy.import_into(&app).await.unwrap();
    let again = legacy.import_into(&app).await;

    // then
    assert_eq!(
        (
            report.users,
            report.admins,
            report.workspaces,
            report.projects,
            report.secrets,
            report.pipelines
        ),
        (2, 1, 1, 1, 1, 1)
    );
    assert_eq!(report.skipped.len(), 3, "{report}");
    assert!(again.is_err());
    assert!(app.sign_in("alice@example.com", PASSWORD).await.is_ok());
    assert!(app.sign_in("bob@example.com", PASSWORD).await.is_err());

    // Check the places: alice owns Acme, bob reads it and edits the project.
    let rows: Vec<(uuid::Uuid, String)> =
        sqlx::query_as("SELECT u.id, u.handle FROM users u ORDER BY u.handle")
            .fetch_all(&app.pool)
            .await
            .unwrap();
    let (alice, bob) = (
        scylla_domain::identity::actors::ActorId::from_db(rows[0].0),
        scylla_domain::identity::actors::ActorId::from_db(rows[1].0),
    );
    let acme: uuid::Uuid = sqlx::query_scalar("SELECT id FROM workspaces WHERE slug = 'acme'")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let acme = WorkspaceId::from_db(acme);
    let owner: uuid::Uuid =
        sqlx::query_scalar("SELECT user_id FROM workspace_owners WHERE workspace_id = $1")
            .bind(acme.to_db())
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(owner, alice.to_db());
    sqlx::query("UPDATE users SET status = 'active' WHERE id = $1")
        .bind(bob.to_db())
        .execute(&app.pool)
        .await
        .unwrap();
    assert!(app.can(bob, Workspace::GET_WORKSPACE.on(acme)).await);
    assert!(!app.can(bob, Workspace::UPDATE_WORKSPACE.on(acme)).await);
    let project = ProjectId::from_db(id_of("01hzzzzzzzzzzzzzzzzzzzzzp1"));
    assert!(
        app.can(bob, Project::UPDATE_PROJECT.on(project).within(acme))
            .await
    );

    // Check the secret and the pipeline.
    let (secret, sealed): (uuid::Uuid, Vec<u8>) =
        sqlx::query_as("SELECT id, sealed_value FROM secrets")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    let opened = app
        .sealer
        .as_ref()
        .unwrap()
        .open_with(&sealed, &secret.into_bytes())
        .unwrap();
    assert_eq!(opened, b"s3cr3t");
    let pipeline = app
        .get_pipeline(alice, &id_of("01hzzzzzzzzzzzzzzzzzzzzzm1").to_string())
        .await
        .unwrap();
    let needs: Vec<_> = pipeline.nodes[1]
        .needs
        .iter()
        .map(|need| (need.node.as_str(), need.port.as_str()))
        .collect();
    assert_eq!(needs, vec![("build", "")]);
    assert_eq!(pipeline.nodes[1].working_dir, "site");

    legacy.delete(&pool).await;
}
