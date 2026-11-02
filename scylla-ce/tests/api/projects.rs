//! The tests of `ProjectService`.

use scylla_proto::projects::v1::{CreateProjectRequest, ListProjectsRequest};
use scylla_testkit::{as_actor, id, text};
use sqlx::PgPool;
use tonic::{Code, Request};

use crate::{app, assert_invalid_field, UNKNOWN_ID};

/// A created project comes back trimmed, owned and created by the caller.
#[sqlx::test(migrations = false)]
async fn create_project_returns_the_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;

    // when
    let project = app.create_project(actor, "  alpha  ").await.unwrap();

    // then
    assert_eq!(project.name, "alpha");
    assert_eq!(text(&project.owner_id), actor.to_string());
    assert_eq!(text(&project.created_by), actor.to_string());
}

/// An actor can create a project for another actor: the author stays the caller.
#[sqlx::test(migrations = false)]
async fn create_project_for_another_owner_keeps_the_author(pool: PgPool) {
    // given
    let app = app(pool).await;
    let author = app.actor().await;
    let owner = app.actor().await;
    let request = CreateProjectRequest {
        owner_id: Some(id(owner.to_string())),
        name: "alpha".to_owned(),
    };

    // when
    let response = app
        .projects
        .clone()
        .create_project(as_actor(author, request))
        .await
        .unwrap();

    // then
    let project = response.into_inner().project.unwrap();
    assert_eq!(text(&project.owner_id), owner.to_string());
    assert_eq!(text(&project.created_by), author.to_string());
}

/// A request with no actor, or with an unknown actor, is refused.
#[sqlx::test(migrations = false)]
async fn create_project_needs_an_actor(pool: PgPool) {
    // given
    let app = app(pool).await;
    let request = || CreateProjectRequest {
        owner_id: None,
        name: "alpha".to_owned(),
    };
    let unknown = scylla_domain_id(UNKNOWN_ID);

    // when
    let anonymous = app
        .projects
        .clone()
        .create_project(Request::new(request()))
        .await;
    let stranger = app
        .projects
        .clone()
        .create_project(as_actor(unknown, request()))
        .await;

    // then
    assert_eq!(anonymous.unwrap_err().code(), Code::Unauthenticated);
    assert_eq!(stranger.unwrap_err().code(), Code::Unauthenticated);
}

/// A name of `MAX_LEN` characters passes, one more fails, and an empty name fails.
#[sqlx::test(migrations = false)]
async fn create_project_checks_the_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;

    // when
    let longest = app.create_project(actor, &"x".repeat(100)).await;
    let too_long = app.create_project(actor, &"x".repeat(101)).await;
    let empty = app.create_project(actor, "   ").await;

    // then
    assert!(longest.is_ok());
    assert_invalid_field(&too_long.unwrap_err(), "name");
    assert_invalid_field(&empty.unwrap_err(), "name");
}

/// An owner must be an id of an existing actor.
#[sqlx::test(migrations = false)]
async fn create_project_checks_the_owner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;
    let request = |owner: &str| CreateProjectRequest {
        owner_id: Some(id(owner)),
        name: "alpha".to_owned(),
    };
    let mut client = app.projects.clone();

    // when
    let not_an_id = client
        .create_project(as_actor(actor, request("nope")))
        .await;
    let unknown = client
        .create_project(as_actor(actor, request(UNKNOWN_ID)))
        .await;

    // then
    assert_invalid_field(&not_an_id.unwrap_err(), "owner_id");
    assert_eq!(unknown.unwrap_err().code(), Code::NotFound);
}

/// The Community edition has no project limit.
#[sqlx::test(migrations = false)]
async fn create_project_has_no_limit(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;

    // when
    for i in 0..20 {
        app.create_project(actor, &format!("p{i}")).await.unwrap();
    }

    // then
    assert_eq!(app.project_count(actor).await, 20);
}

/// The pages give each project one time, oldest first, and the last page has no token.
#[sqlx::test(migrations = false)]
async fn list_projects_returns_pages_in_creation_order(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;
    let mut created = Vec::new();
    for i in 0..20 {
        let project = app.create_project(actor, &format!("p{i}")).await.unwrap();
        created.push(text(&project.id).to_owned());
    }

    // when
    let (mut listed, mut sizes, mut token) = (Vec::new(), Vec::new(), String::new());
    loop {
        let response = app.list_projects(actor, 8, &token).await.unwrap();
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

/// A list gives only the projects of its owner.
#[sqlx::test(migrations = false)]
async fn list_projects_returns_only_the_projects_of_the_owner(pool: PgPool) {
    // given
    let app = app(pool).await;
    let alice = app.actor().await;
    let bob = app.actor().await;
    app.create_project(alice, "alpha").await.unwrap();
    app.create_project(bob, "beta").await.unwrap();

    // when
    let response = app.list_projects(alice, 0, "").await.unwrap();

    // then
    let names: Vec<_> = response.projects.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, vec!["alpha"]);
}

/// A list with no page gives the first page with the default size.
#[sqlx::test(migrations = false)]
async fn list_projects_accepts_a_request_with_no_page(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;
    app.create_project(actor, "alpha").await.unwrap();
    let request = ListProjectsRequest {
        owner_id: None,
        page: None,
    };

    // when
    let response = app
        .projects
        .clone()
        .list_projects(as_actor(actor, request))
        .await
        .unwrap();

    // then
    assert_eq!(response.into_inner().projects.len(), 1);
}

/// A negative page size and a token that the server did not make are refused.
#[sqlx::test(migrations = false)]
async fn list_projects_checks_the_page(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;

    // when
    let negative = app.list_projects(actor, -1, "").await;
    let bad_token = app.list_projects(actor, 10, "nope").await;

    // then
    assert_invalid_field(&negative.unwrap_err(), "page.size");
    assert_invalid_field(&bad_token.unwrap_err(), "page.token");
}

/// A deleted project is no longer counted.
#[sqlx::test(migrations = false)]
async fn delete_project_removes_the_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;
    let project = app.create_project(actor, "alpha").await.unwrap();

    // when
    app.delete_project(actor, text(&project.id)).await.unwrap();

    // then
    assert_eq!(app.project_count(actor).await, 0);
}

/// A delete needs an id, and a project with this id.
#[sqlx::test(migrations = false)]
async fn delete_project_checks_the_id(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;

    // when
    let not_an_id = app.delete_project(actor, "nope").await;
    let unknown = app.delete_project(actor, UNKNOWN_ID).await;

    // then
    assert_invalid_field(&not_an_id.unwrap_err(), "project_id");
    assert_eq!(unknown.unwrap_err().code(), Code::NotFound);
}

/// Reads an actor id from its text form.
fn scylla_domain_id(text: &str) -> scylla_domain::actors::ActorId {
    scylla_domain::actors::ActorId::parse(text).unwrap()
}
