//! The tests of the repositories that a connection sees for a project to create: the request
//! names the workspace in place of a project.

use scylla_proto::integrations::v1::ListConnectionRepositoriesRequest;
use scylla_testkit::{id, Role};
use sqlx::PgPool;
use tonic::Code;

use super::{app_with_fake, permissions};
use crate::assert_code;

/// The request for the repositories of `connection`, for a project to create in `workspace`.
fn for_workspace(workspace: &str, connection: &str) -> ListConnectionRepositoriesRequest {
    ListConnectionRepositoriesRequest {
        workspace_id: Some(id(workspace)),
        connection_id: Some(id(connection)),
        ..ListConnectionRepositoriesRequest::default()
    }
}

/// A member of the workspace reads the repositories of its connection before the project exists.
/// A viewer, a stranger and a connection of another workspace are denied.
#[sqlx::test(migrations = false)]
async fn list_connection_repositories_of_a_workspace_for_a_new_project(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let (member, viewer, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace_id = app.workspace_of(owner).await;
    let workspace = workspace_id.to_string();
    for (actor, role) in [(member, "member"), (viewer, "viewer")] {
        let grant = Role::on("workspace", &workspace, role);
        app.grant_role(owner, grant, &actor.to_string())
            .await
            .unwrap();
    }
    let connection = app
        .insert_github_connection(workspace_id, owner, 7, "acme")
        .await;
    let theirs = app.workspace_of(stranger).await;
    let foreign = app
        .insert_github_connection(theirs, stranger, 8, "other")
        .await;
    let repositories = [(1, "acme/api", false), (2, "acme/web", true)];
    fake.answer_github_install(7, "acme", permissions(), &repositories);
    let list = |actor, connection| {
        app.send_list_connection_repositories(actor, for_workspace(&workspace, connection))
    };

    // when
    let read = list(member, &connection).await.unwrap();
    let viewed = list(viewer, &connection).await;
    let outside = list(stranger, &connection).await;
    let crossed = list(owner, &foreign).await;

    // then
    let names: Vec<_> = read
        .repositories
        .iter()
        .map(|r| r.full_name.as_str())
        .collect();
    assert_eq!(names, ["acme/api", "acme/web"]);
    assert_code(viewed, Code::PermissionDenied);
    assert_code(outside, Code::PermissionDenied);
    assert_code(crossed, Code::PermissionDenied);
}

/// A request names a project or a workspace: both, or none, is not valid.
#[sqlx::test(migrations = false)]
async fn list_connection_repositories_needs_a_project_or_a_workspace(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace_id = app.workspace_of(owner).await;
    let workspace = workspace_id.to_string();
    let connection = app
        .insert_github_connection(workspace_id, owner, 7, "acme")
        .await;
    let project = app.create_project(owner, "API").await.unwrap().id;
    let both = ListConnectionRepositoriesRequest {
        project_id: project,
        ..for_workspace(&workspace, &connection)
    };
    let none = ListConnectionRepositoriesRequest {
        workspace_id: None,
        ..for_workspace(&workspace, &connection)
    };

    // when
    let both = app.send_list_connection_repositories(owner, both).await;
    let none = app.send_list_connection_repositories(owner, none).await;

    // then
    assert_code(both, Code::InvalidArgument);
    assert_code(none, Code::InvalidArgument);
}
