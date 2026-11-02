//! The tests of `ProjectService`: data, validation and permissions.

use scylla_domain::workspaces::Workspace;
use scylla_proto::projects::v1::CreateProjectRequest;
use scylla_testkit::{id, text};
use sqlx::PgPool;
use tonic::{Code, Request};

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Makes a creation request.
fn create(workspace: Option<String>, name: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        workspace_id: workspace.map(id),
        name: name.to_owned(),
    }
}

/// A created project comes back trimmed, in its workspace, and created by the caller.
#[sqlx::test(migrations = false)]
async fn create_project_returns_the_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let project = app.create_project(user, "  alpha  ").await.unwrap();

    // then
    assert_eq!(project.name, "alpha");
    let workspace = app.workspace_of(user).await;
    assert_eq!(text(&project.workspace_id), workspace.to_string());
    assert_eq!(text(&project.created_by), user.to_string());
}

/// A request with no token is refused.
#[sqlx::test(migrations = false)]
async fn create_project_needs_a_token(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let result = app
        .projects
        .clone()
        .create_project(Request::new(create(None, "alpha")))
        .await;

    // then
    assert_code(result, Code::Unauthenticated);
}

/// An actor with no role in a workspace, or a viewer, cannot create a project in it.
#[sqlx::test(migrations = false)]
async fn create_project_needs_a_role_in_the_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let viewer = app.user().await;
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::VIEWER.of(workspace, viewer)]).await;

    // when
    let by_stranger = app.create_project_in(stranger, workspace, "alpha").await;
    let by_viewer = app.create_project_in(viewer, workspace, "alpha").await;

    // then
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(by_viewer, Code::PermissionDenied);
    assert_eq!(app.project_count(workspace).await, 0);
}

/// A member creates in the workspace of another user, and stays the author.
#[sqlx::test(migrations = false)]
async fn create_project_by_a_member_keeps_the_author(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let member = app.user().await;
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;

    // when
    let project = app
        .create_project_in(member, workspace, "alpha")
        .await
        .unwrap();

    // then
    assert_eq!(text(&project.workspace_id), workspace.to_string());
    assert_eq!(text(&project.created_by), member.to_string());
}

/// A name of `MAX_LEN` characters passes, one more fails, and an empty name fails.
#[sqlx::test(migrations = false)]
async fn create_project_checks_the_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let longest = app.create_project(user, &"x".repeat(100)).await;
    let too_long = app.create_project(user, &"x".repeat(101)).await;
    let empty = app.create_project(user, "   ").await;

    // then
    assert!(longest.is_ok());
    assert_invalid_field(&too_long.unwrap_err(), "name");
    assert_invalid_field(&empty.unwrap_err(), "name");
}

/// The workspace is required and must be an id. An unknown workspace gives the answer of a
/// refusal.
#[sqlx::test(migrations = false)]
async fn create_project_checks_the_workspace_id(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let mut client = app.projects.clone();

    // when
    let missing = client
        .create_project(app.authed(user, create(None, "alpha")).await)
        .await;
    let not_an_id = client
        .create_project(
            app.authed(user, create(Some("nope".to_owned()), "alpha"))
                .await,
        )
        .await;
    let unknown = client
        .create_project(
            app.authed(user, create(Some(UNKNOWN_ID.to_owned()), "alpha"))
                .await,
        )
        .await;

    // then
    assert_invalid_field(&missing.unwrap_err(), "workspace_id");
    assert_invalid_field(&not_an_id.unwrap_err(), "workspace_id");
    assert_code(unknown, Code::PermissionDenied);
}

/// The pages give each project one time, oldest first, and the last page has no token.
#[sqlx::test(migrations = false)]
async fn list_projects_returns_pages_in_creation_order(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let mut created = Vec::new();
    for i in 0..20 {
        let project = app.create_project(user, &format!("p{i}")).await.unwrap();
        created.push(text(&project.id).to_owned());
    }

    // when
    let (mut listed, mut sizes, mut token) = (Vec::new(), Vec::new(), String::new());
    loop {
        let response = app.list_projects(user, 8, &token).await.unwrap();
        sizes.push(response.projects.len());
        listed.extend(response.projects.iter().map(|p| text(&p.id).to_owned()));
        token = response.page.unwrap().next_token;
        if token.is_empty() {
            break;
        }
    }

    // then
    assert_eq!(sizes, vec![8, 8, 4]);
    assert_eq!(listed, created);
}

/// The list shows only the projects that the caller can see. `workspace_id` filters the list and
/// causes no refusal.
#[sqlx::test(migrations = false)]
async fn list_projects_shows_only_visible_projects(pool: PgPool) {
    // given
    let app = app(pool).await;
    let alice = app.user().await;
    let bob = app.user().await;
    app.create_project(alice, "alice").await.unwrap();
    app.create_project(bob, "bob").await.unwrap();

    // when
    let all = app.visible_projects(alice, None).await.unwrap();
    let of_bob = app
        .visible_projects(alice, Some(app.workspace_of(bob).await))
        .await
        .unwrap();

    // then
    assert_eq!(all, vec!["alice"]);
    assert!(of_bob.is_empty(), "{of_bob:?}");
}

/// The owner gets its project.
#[sqlx::test(migrations = false)]
async fn get_project_returns_the_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let created = app.create_project(user, "alpha").await.unwrap();

    // when
    let project = app.get_project(user, text(&created.id)).await.unwrap();

    // then
    assert_eq!(project, created);
}

/// Another actor cannot get a project, and an unknown project gives the same answer.
#[sqlx::test(migrations = false)]
async fn get_project_needs_the_permission(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();

    // when
    let by_stranger = app.get_project(stranger, text(&project.id)).await;
    let unknown = app.get_project(owner, UNKNOWN_ID).await;
    let not_an_id = app.get_project(owner, "nope").await;

    // then
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_invalid_field(&not_an_id.unwrap_err(), "project_id");
}

/// A negative page size and a token that the server did not make are refused.
#[sqlx::test(migrations = false)]
async fn list_projects_checks_the_page(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let negative = app.list_projects(user, -1, "").await;
    let bad_token = app.list_projects(user, 10, "nope").await;

    // then
    assert_invalid_field(&negative.unwrap_err(), "page.size");
    assert_invalid_field(&bad_token.unwrap_err(), "page.token");
}

/// The owner deletes its project.
#[sqlx::test(migrations = false)]
async fn delete_project_removes_the_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();

    // when
    app.delete_project(user, text(&project.id)).await.unwrap();

    // then
    assert_eq!(app.project_count(app.workspace_of(user).await).await, 0);
}

/// Another actor cannot delete a project, and an unknown project gives the same answer.
#[sqlx::test(migrations = false)]
async fn delete_project_needs_the_permission(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let project = app.create_project(owner, "alpha").await.unwrap();

    // when
    let by_stranger = app.delete_project(stranger, text(&project.id)).await;
    let unknown = app.delete_project(owner, UNKNOWN_ID).await;
    let not_an_id = app.delete_project(owner, "nope").await;

    // then
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_invalid_field(&not_an_id.unwrap_err(), "project_id");
    assert_eq!(app.project_count(app.workspace_of(owner).await).await, 1);
}
