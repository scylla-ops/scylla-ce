//! The tests of ownership: an owner needs no grant and no SpiceDB, and a grant on an owner goes
//! down to what it owns.

use scylla_domain::actors::Actor;
use scylla_proto::projects::v1::{CreateProjectRequest, DeleteProjectRequest, Project};
use scylla_testkit::{id, text, with_token, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};

use crate::agents::agent_of;
use crate::{app, assert_code};

/// Calls `CreateProject` with a token, for the actor of the token.
async fn create_as(app: &TestApp, token: &str) -> Project {
    let request = CreateProjectRequest {
        owner_id: None,
        name: "alpha".to_owned(),
    };
    let response = app
        .projects
        .clone()
        .create_project(with_token(token, request))
        .await
        .unwrap();
    response.into_inner().project.unwrap()
}

/// Calls `DeleteProject` with a token.
async fn delete_as(app: &TestApp, token: &str, project: &Project) -> Result<(), Status> {
    let request = DeleteProjectRequest {
        project_id: Some(id(text(&project.id))),
    };
    app.projects
        .clone()
        .delete_project(with_token(token, request))
        .await?;
    Ok(())
}

/// A user deletes a project of its agent, but the agent cannot delete a project of its user.
#[sqlx::test(migrations = false)]
async fn an_owner_acts_on_what_its_agents_own(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let (agent, agent_token) = agent_of(&app, user).await;
    let agent_project = create_as(&app, &agent_token).await;
    let user_project = app.create_project(user, "own").await.unwrap();

    // when
    let by_agent = delete_as(&app, &agent_token, &user_project).await;
    let by_user = app.delete_project(user, text(&agent_project.id)).await;

    // then
    assert_eq!(text(&agent_project.owner_id), agent.to_string());
    assert_code(by_agent, Code::PermissionDenied);
    assert!(by_user.is_ok(), "{by_user:?}");
}

/// A delegate of a user deletes a project of the agent of the user: a grant on an owner goes
/// down to what the owner owns.
#[sqlx::test(migrations = false)]
async fn a_grant_on_an_owner_goes_down(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let delegate = app.user().await;
    let stranger = app.user().await;
    let (_, agent_token) = agent_of(&app, user).await;
    let project = create_as(&app, &agent_token).await;
    app.grant(&[Actor::DELEGATE.of(user, delegate)]).await;

    // when
    let by_stranger = app.delete_project(stranger, text(&project.id)).await;
    let by_delegate = app.delete_project(delegate, text(&project.id)).await;

    // then
    assert_code(by_stranger, Code::PermissionDenied);
    assert!(by_delegate.is_ok(), "{by_delegate:?}");
}

/// An owner creates, lists and deletes its projects with no SpiceDB. Only a grant needs it.
#[sqlx::test(migrations = false)]
async fn an_owner_needs_no_spicedb(pool: PgPool) {
    // given
    let app = TestApp::spawn_without_spicedb(pool, scylla_ce::edition()).await;
    let owner = app.user().await;
    let delegate = app.user().await;
    let for_owner = CreateProjectRequest {
        owner_id: Some(id(owner.to_string())),
        name: "beta".to_owned(),
    };

    // when
    let created = app.create_project(owner, "alpha").await;
    let listed = app.list_projects(owner, 10, "").await;
    let project_id = created
        .as_ref()
        .map(|project| text(&project.id).to_owned())
        .unwrap_or_default();
    let deleted = app.delete_project(owner, &project_id).await;
    let by_delegate = app
        .projects
        .clone()
        .create_project(app.authed(delegate, for_owner).await)
        .await;

    // then
    assert!(created.is_ok(), "{created:?}");
    assert!(listed.is_ok(), "{listed:?}");
    assert!(deleted.is_ok(), "{deleted:?}");
    assert_code(by_delegate, Code::Unavailable);
}
