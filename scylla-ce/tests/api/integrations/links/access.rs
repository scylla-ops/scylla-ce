//! The tests of the access to the repository link of a project: the readers read it, only the
//! editors change it, and a connection of another workspace is refused.

use scylla_proto::integrations::v1::UpdateRepositoryLinkRequest;
use scylla_testkit::Role;
use sqlx::PgPool;
use tonic::Code;

use super::{app_with_fake, assert_reason, insert_link, project};
use crate::assert_code;

/// A viewer of the project reads the link and its refs, but does not change the link. An editor
/// changes it. A stranger reads nothing.
#[sqlx::test(migrations = false)]
async fn only_an_editor_changes_the_link(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let project = project(&app, "github").await;
    insert_link(&app, &project, "octo/app").await;
    fake.answer_github_refs(42, "octo/app", &[("main", "a1")], &[]);
    let (viewer, editor, stranger) = (app.user().await, app.user().await, app.user().await);
    for (actor, role) in [(viewer, "viewer"), (editor, "editor")] {
        app.join_workspace(project.owner, actor).await;
        let grant = Role::on("project", &project.id, role);
        app.grant_role(project.owner, grant, &actor.to_string())
            .await
            .unwrap();
    }
    let changes = || UpdateRepositoryLinkRequest {
        reports_enabled: Some(false),
        ..UpdateRepositoryLinkRequest::default()
    };
    let kind = scylla_proto::integrations::v1::GitRefKind::Unspecified;

    // when
    let viewer_get = app.get_repository_link(viewer, &project.id).await;
    let viewer_refs = app.list_refs(viewer, &project.id, kind, "", 0, "").await;
    let viewer_link = app
        .link_repository(viewer, &project.id, &project.connection, "7")
        .await;
    let viewer_update = app
        .update_repository_link(viewer, &project.id, changes())
        .await;
    let viewer_test = app.test_delivery(viewer, &project.id).await;
    let viewer_unlink = app.unlink_repository(viewer, &project.id).await;
    let stranger_get = app.get_repository_link(stranger, &project.id).await;
    let editor_update = app
        .update_repository_link(editor, &project.id, changes())
        .await;

    // then
    assert!(viewer_get.unwrap().is_some());
    assert_eq!(viewer_refs.unwrap().refs.len(), 1);
    assert_code(viewer_link, Code::PermissionDenied);
    assert_code(viewer_update, Code::PermissionDenied);
    assert_code(viewer_test, Code::PermissionDenied);
    assert_code(viewer_unlink, Code::PermissionDenied);
    assert_code(stranger_get, Code::PermissionDenied);
    assert!(!editor_update.unwrap().reports_enabled);
}

/// A connection of another workspace is refused as an unknown one, and an unknown project gets
/// the answer of a refusal.
#[sqlx::test(migrations = false)]
async fn link_repository_refuses_a_connection_of_another_workspace(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let ours = project(&app, "github").await;
    let theirs = project(&app, "github").await;

    // when
    let foreign = app
        .link_repository(ours.owner, &ours.id, &theirs.connection, "7")
        .await;
    let unknown = app
        .link_repository(ours.owner, crate::UNKNOWN_ID, &ours.connection, "7")
        .await;

    // then
    assert_code(foreign, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert!(fake.requests().is_empty());
}

/// The test of a webhook needs a link. A refusal of the provider is a failed test, with no
/// wait.
#[sqlx::test(migrations = false)]
async fn test_delivery_needs_a_link(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = project(&app, "github").await;
    insert_link(&app, &linked, "octo/app").await;
    let unlinked = project(&app, "github").await;

    // when
    let refused = app.test_delivery(linked.owner, &linked.id).await;
    let none = app.test_delivery(unlinked.owner, &unlinked.id).await;

    // then
    let refused = refused.unwrap();
    assert!(!refused.delivered);
    assert!(refused.error.starts_with("GitHub: "), "{}", refused.error);
    assert_reason(none, "NO_REPOSITORY_LINK");
}
