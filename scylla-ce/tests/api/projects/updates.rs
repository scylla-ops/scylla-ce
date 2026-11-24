//! The tests of `UpdateProject` and of the description of a project.

use scylla_proto::projects::v1::{CreateProjectRequest, UpdateProjectRequest};
use scylla_testkit::{id, text};
use sqlx::PgPool;
use tonic::Code;

use super::cast::{passed, Cast};
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// A change of the name and of the description comes back trimmed, and `GetProject` reads it.
#[sqlx::test(migrations = false)]
async fn update_project_changes_the_name_and_the_description(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let created = app.create_project(user, "alpha").await.unwrap();
    let id = text(&created.id);

    // when
    let updated = app
        .update_project(user, id, Some("  beta  "), Some("  line one\nline two  "))
        .await
        .unwrap();

    // then
    assert_eq!(updated.name, "beta");
    assert_eq!(updated.description, "line one\nline two");
    assert_eq!(updated.created_at, created.created_at);
    assert_eq!(app.get_project(user, id).await.unwrap(), updated);
}

/// A field that is not set does not change, and an empty description removes the description.
#[sqlx::test(migrations = false)]
async fn update_project_keeps_the_fields_that_are_not_set(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let id = text(&project.id);
    app.update_project(user, id, None, Some("about alpha"))
        .await
        .unwrap();

    // when
    let renamed = app
        .update_project(user, id, Some("beta"), None)
        .await
        .unwrap();
    let nothing = app.update_project(user, id, None, None).await.unwrap();
    let cleared = app
        .update_project(user, id, None, Some("  "))
        .await
        .unwrap();

    // then
    assert_eq!(
        (renamed.name.as_str(), renamed.description.as_str()),
        ("beta", "about alpha")
    );
    assert_eq!(nothing, renamed);
    assert_eq!(
        (cleared.name.as_str(), cleared.description.as_str()),
        ("beta", "")
    );
}

/// A name of 100 characters passes and 101 fails. An empty name and a name with a line break
/// fail.
#[sqlx::test(migrations = false)]
async fn update_project_checks_the_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let id = text(&project.id);

    // when
    let longest = app
        .update_project(user, id, Some(&"é".repeat(100)), None)
        .await;
    let too_long = app
        .update_project(user, id, Some(&"é".repeat(101)), None)
        .await;
    let empty = app.update_project(user, id, Some("  "), None).await;
    let two_lines = app.update_project(user, id, Some("a\nb"), None).await;

    // then
    assert_eq!(longest.unwrap().name, "é".repeat(100));
    assert_invalid_field(&too_long.unwrap_err(), "name");
    assert_invalid_field(&empty.unwrap_err(), "name");
    assert_invalid_field(&two_lines.unwrap_err(), "name");
}

/// A description of 1024 characters passes and 1025 fails, on `UpdateProject` and on
/// `CreateProject`.
#[sqlx::test(migrations = false)]
async fn the_description_has_at_most_1024_characters(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let id = text(&project.id);
    let create = |description: String| CreateProjectRequest {
        workspace_id: project.workspace_id.clone(),
        name: "beta".to_owned(),
        description,
    };

    // when
    let longest = app
        .update_project(user, id, None, Some(&"é".repeat(1024)))
        .await;
    let too_long = app
        .update_project(user, id, None, Some(&"é".repeat(1025)))
        .await;
    let mut client = app.projects.clone();
    let created = client
        .create_project(app.authed(user, create("x".repeat(1024))).await)
        .await;
    let refused = client
        .create_project(app.authed(user, create("x".repeat(1025))).await)
        .await;

    // then
    assert_eq!(longest.unwrap().description, "é".repeat(1024));
    assert_invalid_field(&too_long.unwrap_err(), "description");
    let created = created.unwrap().into_inner().project.unwrap();
    assert_eq!(created.description, "x".repeat(1024));
    assert_invalid_field(&refused.unwrap_err(), "description");
}

/// An unknown project gives the answer of a refusal, and the id is required.
#[sqlx::test(migrations = false)]
async fn update_project_needs_a_known_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let mut client = app.projects.clone();
    let missing = UpdateProjectRequest {
        project_id: None,
        name: Some("beta".to_owned()),
        description: None,
    };

    // when
    let unknown = app
        .update_project(user, UNKNOWN_ID, Some("beta"), None)
        .await;
    let not_an_id = app.update_project(user, "nope", Some("beta"), None).await;
    let missing = client.update_project(app.authed(user, missing).await).await;

    // then
    assert_code(unknown, Code::PermissionDenied);
    assert_invalid_field(&not_an_id.unwrap_err(), "project_id");
    assert_invalid_field(&missing.unwrap_err(), "project_id");
}

/// The roles that edit pass: the owner, the admins and the members of the workspace, and the
/// admins and the editors of the project. The viewers and a stranger get `PERMISSION_DENIED`.
#[sqlx::test(migrations = false)]
async fn update_project_needs_a_role_that_edits(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;

    // when
    let mut results = Vec::new();
    for (role, actor) in cast.actors {
        let name = format!("by {role}");
        let result = app
            .update_project(actor, &cast.project, Some(&name), None)
            .await;
        results.push((role, result));
    }

    // then
    let editors = [
        "owner",
        "workspace admin",
        "member",
        "project admin",
        "editor",
    ];
    assert_eq!(passed(results), editors);
    let project = app.get_project(cast.owner(), &cast.project).await.unwrap();
    assert_eq!(project.name, "by editor");
}

/// A creation with a description keeps it trimmed.
#[sqlx::test(migrations = false)]
async fn create_project_keeps_the_description(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let request = CreateProjectRequest {
        workspace_id: Some(id(app.workspace_of(user).await.to_string())),
        name: "alpha".to_owned(),
        description: "  about alpha \n".to_owned(),
    };

    // when
    let response = app
        .projects
        .clone()
        .create_project(app.authed(user, request).await)
        .await
        .unwrap();

    // then
    let project = response.into_inner().project.unwrap();
    assert_eq!(project.description, "about alpha");
}
