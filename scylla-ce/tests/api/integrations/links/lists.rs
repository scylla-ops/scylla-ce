//! The tests of `ListRepositoryLinks`: the links of the projects of a workspace in one call, with
//! the filter of the projects and the access of the caller.

use scylla_testkit::{text, Role};
use sqlx::PgPool;
use tonic::Code;

use super::{app_with_fake, insert_link, project, Project};
use crate::assert_code;

/// The list gives the link of each linked project of the workspace, oldest project first, and
/// only the projects of the filter when it has ids. A project with no link has no item.
#[sqlx::test(migrations = false)]
async fn list_repository_links_gives_the_links_of_a_workspace(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let first = project(&app, "github").await;
    insert_link(&app, &first, "octo/app").await;
    let second = app.create_project(first.owner, "Web").await.unwrap();
    let second = Project {
        id: text(&second.id).to_owned(),
        connection: first.connection.clone(),
        owner: first.owner,
    };
    insert_link(&app, &second, "octo/web").await;
    let unlinked = app.create_project(first.owner, "Docs").await.unwrap();
    let workspace = app.workspace_of(first.owner).await.to_string();

    // when
    let all = app
        .list_repository_links(first.owner, &workspace, &[])
        .await
        .unwrap();
    let filtered = app
        .list_repository_links(first.owner, &workspace, &[&second.id, text(&unlinked.id)])
        .await
        .unwrap();

    // then
    let full_names: Vec<_> = all.iter().map(|link| link.full_name.as_str()).collect();
    assert_eq!(full_names, ["octo/app", "octo/web"]);
    assert_eq!(filtered.len(), 1);
    assert_eq!(text(&filtered[0].project_id), second.id);
}

/// A viewer of one project gets only the link of that project, and a stranger gets none: a
/// filter causes no refusal. More than 100 project ids give `INVALID_ARGUMENT`.
#[sqlx::test(migrations = false)]
async fn list_repository_links_shows_only_what_the_caller_reads(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let shared = project(&app, "github").await;
    insert_link(&app, &shared, "octo/app").await;
    let other = app.create_project(shared.owner, "Other").await.unwrap();
    let other = Project {
        id: text(&other.id).to_owned(),
        connection: shared.connection.clone(),
        owner: shared.owner,
    };
    insert_link(&app, &other, "octo/other").await;
    let workspace = app.workspace_of(shared.owner).await.to_string();
    let (viewer, stranger) = (app.user().await, app.user().await);
    app.join_workspace(shared.owner, viewer).await;
    let grant = Role::on("project", &shared.id, "viewer");
    app.grant_role(shared.owner, grant, &viewer.to_string())
        .await
        .unwrap();
    let many: Vec<String> = (0..101).map(|_| shared.id.clone()).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();

    // when
    let of_viewer = app.list_repository_links(viewer, &workspace, &[]).await;
    let of_stranger = app.list_repository_links(stranger, &workspace, &[]).await;
    let too_many = app
        .list_repository_links(shared.owner, &workspace, &many)
        .await;

    // then
    let of_viewer = of_viewer.unwrap();
    assert_eq!(of_viewer.len(), 1);
    assert_eq!(of_viewer[0].full_name, "octo/app");
    assert!(of_stranger.unwrap().is_empty());
    assert_code(too_many, Code::InvalidArgument);
}
