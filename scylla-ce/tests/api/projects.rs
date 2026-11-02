//! The tests of `ProjectService`: data, validation and permissions.

use scylla_extension::Object;
use scylla_proto::projects::v1::{CreateProjectRequest, ListProjectsRequest};
use scylla_testkit::{id, text};
use sqlx::PgPool;
use tonic::{Code, Request};

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Makes a creation request.
fn create(owner: Option<String>, name: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        owner_id: owner.map(id),
        name: name.to_owned(),
    }
}

/// A created project comes back trimmed, owned and created by the caller.
#[sqlx::test(migrations = false)]
async fn create_project_returns_the_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let project = app.create_project(user, "  alpha  ").await.unwrap();

    // then
    assert_eq!(project.name, "alpha");
    assert_eq!(text(&project.owner_id), user.to_string());
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

/// An actor cannot create a project for an owner that it cannot act for.
#[sqlx::test(migrations = false)]
async fn create_project_needs_to_act_for_the_owner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let author = app.user().await;
    let owner = app.user().await;
    let request = create(Some(owner.to_string()), "alpha");

    // when
    let result = app
        .projects
        .clone()
        .create_project(app.authed(author, request).await)
        .await;

    // then
    assert_code(result, Code::PermissionDenied);
}

/// A delegate creates for its owner: the owner owns, and the delegate is the author.
#[sqlx::test(migrations = false)]
async fn create_project_by_a_delegate_keeps_the_author(pool: PgPool) {
    // given
    let app = app(pool).await;
    let author = app.user().await;
    let owner = app.user().await;
    let owner_object = Object::new("actor", owner);
    app.grant(owner_object, "delegate", Object::new("actor", author))
        .await;
    let request = create(Some(owner.to_string()), "alpha");

    // when
    let response = app
        .projects
        .clone()
        .create_project(app.authed(author, request).await)
        .await
        .unwrap();

    // then
    let project = response.into_inner().project.unwrap();
    assert_eq!(text(&project.owner_id), owner.to_string());
    assert_eq!(text(&project.created_by), author.to_string());
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

/// An owner must be an id.
#[sqlx::test(migrations = false)]
async fn create_project_checks_the_owner_id(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let request = create(Some("nope".to_owned()), "alpha");

    // when
    let result = app
        .projects
        .clone()
        .create_project(app.authed(user, request).await)
        .await;

    // then
    assert_invalid_field(&result.unwrap_err(), "owner_id");
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

/// An actor cannot list the projects of another actor.
#[sqlx::test(migrations = false)]
async fn list_projects_needs_to_act_for_the_owner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let alice = app.user().await;
    let bob = app.user().await;
    let request = ListProjectsRequest {
        owner_id: Some(id(bob.to_string())),
        page: None,
    };

    // when
    let result = app
        .projects
        .clone()
        .list_projects(app.authed(alice, request).await)
        .await;

    // then
    assert_code(result, Code::PermissionDenied);
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
    assert_eq!(app.project_count(user).await, 0);
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
    assert_eq!(app.project_count(owner).await, 1);
}
