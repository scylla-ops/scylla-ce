//! The tests of the list of the secrets of a workspace: the roles that see the secrets of a
//! project, the projects that the caller sees, the text filter, the pages, and a bad request.

use scylla_domain::identity::actors::ActorId;
use scylla_testkit::{Role, TestApp};
use sqlx::PgPool;

use super::cast::Cast;
use super::job_stats::{place, Place};
use crate::{app, assert_invalid_field};

/// The roles that see the secrets of the project in the list of its workspace.
const READERS: [&str; 5] = [
    "owner",
    "workspace admin",
    "member",
    "project admin",
    "editor",
];

/// Returns the names of the first 100 secrets of the workspace of a place that match a text, as
/// a caller, newest first, each with the name of its project.
async fn names(
    app: &TestApp,
    place: &Place,
    caller: ActorId,
    query: &str,
) -> Vec<(String, String)> {
    let response = app.list_workspace_secrets(caller, place.workspace, query, 100, "");
    let secrets = response.await.unwrap().secrets;
    secrets
        .into_iter()
        .map(|found| (found.secret.unwrap().name, found.project_name))
        .collect()
}

/// Creates the secrets `DB_PASSWORD` in `alpha`, then `API_TOKEN` and `db_replica` in `beta`.
async fn secrets(app: &TestApp, place: &Place) {
    let made = [(0, "DB_PASSWORD"), (1, "API_TOKEN"), (1, "db_replica")];
    for (project, name) in made {
        let created = app.create_secret(place.owner, &place.projects[project], name, "v");
        created.await.unwrap();
    }
}

/// Only the roles that list the secrets of the project see them in the list of the workspace.
/// The other callers get an empty list.
#[sqlx::test(migrations = false)]
async fn list_workspace_secrets_shows_the_projects_where_the_caller_lists_secrets(pool: PgPool) {
    // given
    let app = app(pool).await;
    let cast = Cast::new(&app).await;
    let created = app.create_secret(cast.owner(), &cast.project, "TOKEN", "v");
    created.await.unwrap();
    let workspace = app.workspace_of(cast.owner()).await;

    // when
    let mut readers = Vec::new();
    for (role, actor) in cast.actors {
        let page = app.list_workspace_secrets(actor, workspace, "", 10, "");
        let found = page.await.unwrap().secrets;
        if !found.is_empty() {
            assert_eq!(found[0].secret.as_ref().unwrap().name, "TOKEN", "{role}");
            assert_eq!(found[0].project_name, "alpha", "{role}");
            readers.push(role);
        }
    }

    // then
    assert_eq!(readers, READERS);
}

/// An editor of one project sees only the secrets of this project.
#[sqlx::test(migrations = false)]
async fn list_workspace_secrets_shows_only_the_shared_projects(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    secrets(&app, &place).await;
    let editor = app.user().await;
    app.join_workspace(place.owner, editor).await;
    let role = Role::on("project", &place.projects[1], "editor");
    let granted = app.grant_role(place.owner, role, &editor.to_string()).await;
    granted.unwrap();

    // when
    let seen = names(&app, &place, editor, "").await;

    // then
    let beta = |name: &str| (name.to_owned(), "beta".to_owned());
    assert_eq!(seen, vec![beta("db_replica"), beta("API_TOKEN")]);
}

/// The text finds a secret by its name or by the name of its project, in any case.
#[sqlx::test(migrations = false)]
async fn list_workspace_secrets_finds_a_secret_by_its_name_or_its_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    secrets(&app, &place).await;

    // when
    let by_name = names(&app, &place, place.owner, "DB").await;
    let by_project = names(&app, &place, place.owner, " alpha ").await;
    let none = names(&app, &place, place.owner, "nothing").await;

    // then
    let names_of = |found: Vec<(String, String)>| -> Vec<String> {
        found.into_iter().map(|(name, _)| name).collect()
    };
    assert_eq!(names_of(by_name), vec!["db_replica", "DB_PASSWORD"]);
    assert_eq!(names_of(by_project), vec!["DB_PASSWORD"]);
    assert!(none.is_empty());
}

/// The pages give each secret once, newest first, and the last page has no next token.
#[sqlx::test(migrations = false)]
async fn list_workspace_secrets_reads_the_pages(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    secrets(&app, &place).await;

    // when
    let mut seen = Vec::new();
    let mut token = String::new();
    loop {
        let page = app.list_workspace_secrets(place.owner, place.workspace, "", 1, &token);
        let page = page.await.unwrap();
        seen.extend(
            page.secrets
                .into_iter()
                .map(|found| found.secret.unwrap().name),
        );
        token = page.page.unwrap().next_token;
        if token.is_empty() {
            break;
        }
    }

    // then
    assert_eq!(seen, vec!["db_replica", "API_TOKEN", "DB_PASSWORD"]);
}

/// A text of more than 100 characters is a bad request.
#[sqlx::test(migrations = false)]
async fn list_workspace_secrets_refuses_a_long_query(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let long = "x".repeat(101);

    // when
    let page = app.list_workspace_secrets(place.owner, place.workspace, &long, 10, "");
    let status = page.await.expect_err("a long query");

    // then
    assert_invalid_field(&status, "query");
}
