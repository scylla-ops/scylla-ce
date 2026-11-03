//! The tests of the access to the secrets: the roles of each route, an unknown project or
//! secret, the permissions of a caller, and a server with no key.

use scylla_domain::projects::secrets::SecretId;
use scylla_domain::projects::ProjectId;
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use super::cast::{passed, Cast};
use crate::identity::accounts::assert_refused;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// The roles that pass the routes that change the secrets.
const ADMINS: [&str; 3] = ["owner", "workspace admin", "project admin"];

/// The roles that pass `ListSecrets`.
const EDITORS: [&str; 5] = [
    "owner",
    "workspace admin",
    "member",
    "project admin",
    "editor",
];

/// Creates a secret as the owner, and returns its id.
async fn secret(app: &TestApp, cast: &Cast, name: &str) -> String {
    let response = app
        .create_secret(cast.owner(), &cast.project, name, "v")
        .await;
    text(&response.unwrap().secret.unwrap().id).to_owned()
}

/// Only the owner, the admins of the workspace and the admins of the project create a secret.
#[sqlx::test(migrations = false)]
async fn create_secret_needs_an_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;

    // when
    let mut results = Vec::new();
    for (role, actor) in cast.actors {
        let name = role.replace(' ', "_");
        let result = app.create_secret(actor, &cast.project, &name, "v").await;
        results.push((role, result));
    }

    // then
    assert_eq!(passed(results), ADMINS);
    let names = app.secret_names(cast.owner(), &cast.project).await.unwrap();
    assert_eq!(names, ["owner", "project_admin", "workspace_admin"]);
}

/// Only the owner, the admins of the workspace and the admins of the project change a value.
#[sqlx::test(migrations = false)]
async fn update_secret_needs_an_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;
    let id = secret(&app, &cast, "TOKEN").await;

    // when
    let mut results = Vec::new();
    for (role, actor) in cast.actors {
        results.push((role, app.update_secret(actor, &id, role).await));
    }

    // then
    assert_eq!(passed(results), ADMINS);
}

/// Only the owner, the admins of the workspace and the admins of the project delete a secret.
#[sqlx::test(migrations = false)]
async fn delete_secret_needs_an_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;
    let mut ids = Vec::new();
    for (role, _) in cast.actors {
        ids.push(secret(&app, &cast, &role.replace(' ', "_")).await);
    }

    // when
    let mut results = Vec::new();
    for ((role, actor), id) in cast.actors.into_iter().zip(&ids) {
        results.push((role, app.delete_secret(actor, id).await));
    }

    // then
    assert_eq!(passed(results), ADMINS);
    let names = app.secret_names(cast.owner(), &cast.project).await.unwrap();
    assert_eq!(names.len(), 5);
}

/// The owner, the admins and the members of the workspace, and the admins and the editors of
/// the project list the secrets. The viewers and a stranger cannot.
#[sqlx::test(migrations = false)]
async fn list_secrets_needs_a_role_that_edits(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;
    secret(&app, &cast, "TOKEN").await;

    // when
    let mut results = Vec::new();
    for (role, actor) in cast.actors {
        results.push((role, app.secret_names(actor, &cast.project).await));
    }

    // then
    for (_, names) in results.iter().filter(|(_, result)| result.is_ok()) {
        assert_eq!(names.as_ref().unwrap(), &["TOKEN"]);
    }
    assert_eq!(passed(results), EDITORS);
}

/// `CheckPermissions` tells each role its routes on the secrets.
#[sqlx::test(migrations = false)]
async fn check_permissions_tells_the_routes_of_the_secrets(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;
    let names = ["update_project", "create_secret", "list_secrets"];

    // when
    let mut answers = Vec::new();
    for (role, actor) in cast.actors {
        let answer = app.check_permissions(actor, "project", &cast.project, &names);
        answers.push((role, answer.await.unwrap()));
    }

    // then
    for (role, answer) in answers {
        let editor = EDITORS.contains(&role);
        assert_eq!(answer, [editor, ADMINS.contains(&role), editor], "{role}");
    }
}

/// An unknown project or secret gives the answer of a refusal. A refusal on a secret names the
/// secret, not its project.
#[sqlx::test(migrations = false)]
async fn an_unknown_project_or_secret_gets_the_answer_of_a_refusal(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;
    let id = secret(&app, &cast, "TOKEN").await;
    let stranger = cast.actors[7].1;

    // when
    let create = app
        .create_secret(cast.owner(), UNKNOWN_ID, "TOKEN", "v")
        .await;
    let list = app.list_secrets(cast.owner(), UNKNOWN_ID, 10, "").await;
    let unknown = app.update_secret(cast.owner(), UNKNOWN_ID, "v").await;
    let refused = app.update_secret(stranger, &id, "v").await;
    let deleted = app.delete_secret(cast.owner(), UNKNOWN_ID).await;
    let not_an_id = app.delete_secret(cast.owner(), "nope").await;

    // then
    assert_code(create, Code::PermissionDenied);
    assert_code(list, Code::PermissionDenied);
    let unknown = unknown.unwrap_err();
    let refused = refused.unwrap_err();
    assert_eq!(unknown.code(), Code::PermissionDenied);
    assert_eq!(
        unknown.message(),
        format!("update_secret on secret:{UNKNOWN_ID} is denied")
    );
    assert_eq!(refused.code(), Code::PermissionDenied);
    assert_eq!(
        refused.message(),
        format!("update_secret on secret:{id} is denied")
    );
    assert_code(deleted, Code::PermissionDenied);
    assert_invalid_field(&not_an_id.unwrap_err(), "secret_id");
}

/// With no key, the server refuses a new secret and a new value. It still lists and deletes the
/// secrets.
#[sqlx::test(migrations = false)]
async fn a_server_with_no_key_seals_no_secret(pool: PgPool) {
    // given
    let app = TestApp::spawn_without_secret_key(pool, scylla_ce::edition()).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let id = SecretId::generate();
    sqlx::query!(
        "INSERT INTO secrets (id, project_id, name, sealed_value, created_by)
         VALUES ($1, $2, 'OLD', '\\x00', $3)",
        id.to_db(),
        ProjectId::parse(text(&project.id)).unwrap().to_db(),
        user.to_db(),
    )
    .execute(&app.pool)
    .await
    .unwrap();

    // when
    let created = app.create_secret(user, text(&project.id), "NEW", "v").await;
    let updated = app.update_secret(user, &id.to_string(), "v").await;
    let names = app.secret_names(user, text(&project.id)).await.unwrap();
    let deleted = app.delete_secret(user, &id.to_string()).await;

    // then
    assert_refused(created, "SECRET_KEY_MISSING");
    assert_refused(updated, "SECRET_KEY_MISSING");
    assert_eq!(names, ["OLD"]);
    assert!(deleted.is_ok());
}
