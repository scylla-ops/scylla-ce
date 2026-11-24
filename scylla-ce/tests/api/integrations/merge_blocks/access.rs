//! The tests of the access to the merge rules of a project: the readers list them, only the
//! editors change them, and a change needs a linked repository that serves.

use scylla_proto::integrations::v1::MergeBlockState;
use scylla_testkit::{text, Role};
use sqlx::PgPool;
use tonic::Code;

use super::{app_with_fake, assert_reason, linked};
use crate::{assert_code, assert_invalid_field, UNKNOWN_ID};

/// A viewer of the project lists the rules, but does not change them. An editor changes them. A
/// stranger and an unknown project get a refusal.
#[sqlx::test(migrations = false)]
async fn only_an_editor_changes_a_merge_rule(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    fake.answer_gitlab_merge_rule(7, 200);
    fake.answer_gitlab_merge_setting(7, true);
    let (viewer, editor, stranger) = (app.user().await, app.user().await, app.user().await);
    for (actor, role) in [(viewer, "viewer"), (editor, "editor")] {
        app.join_workspace(linked.owner, actor).await;
        let grant = Role::on("project", &linked.project, role);
        app.grant_role(linked.owner, grant, &actor.to_string())
            .await
            .unwrap();
    }
    let project = linked.project.as_str();

    // when
    let viewer_set = app.set_merge_block(viewer, project, None, true).await;
    let editor_set = app.set_merge_block(editor, project, None, true).await;
    let viewer_list = app.list_merge_blocks(viewer, project).await;
    let stranger_list = app.list_merge_blocks(stranger, project).await;
    let stranger_set = app.set_merge_block(stranger, project, None, false).await;
    let unknown = app.list_merge_blocks(linked.owner, UNKNOWN_ID).await;

    // then
    assert_code(viewer_set, Code::PermissionDenied);
    assert_eq!(editor_set.unwrap().state, MergeBlockState::On as i32);
    assert_eq!(viewer_list.unwrap()[0].state, MergeBlockState::On as i32);
    assert_code(stranger_list, Code::PermissionDenied);
    assert_code(stranger_set, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_eq!(fake.requests_to("PUT", "/api/v4/projects/7").len(), 1);
}

/// A project with no link has no rule, and a change gives `NO_REPOSITORY_LINK`. On GitHub, a
/// change needs a pipeline of the project, and a link that serves.
#[sqlx::test(migrations = false)]
async fn set_merge_block_needs_a_linked_repository_that_serves(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let owner = linked.owner;
    let unlinked = app.create_project(owner, "Docs").await.unwrap();
    let unlinked = text(&unlinked.id).to_owned();
    let project = linked.project.as_str();

    // when
    let none = app.list_merge_blocks(owner, &unlinked).await;
    let no_link = app.set_merge_block(owner, &unlinked, None, true).await;
    let no_pipeline = app.set_merge_block(owner, project, None, true).await;
    let unknown = app
        .set_merge_block(owner, project, Some(UNKNOWN_ID), true)
        .await;
    let pipeline = Some(linked.pipeline.as_str());
    app.set_connection_state(&linked.connection, "suspended", "APP_SUSPENDED")
        .await;
    let suspended = app.set_merge_block(owner, project, pipeline, true).await;
    let listed = app.list_merge_blocks(owner, project).await;

    // then
    assert!(none.unwrap().is_empty());
    assert_reason(no_link, "NO_REPOSITORY_LINK");
    assert_invalid_field(&no_pipeline.unwrap_err(), "pipeline_id");
    assert_code(unknown, Code::NotFound);
    assert_reason(suspended, "REPOSITORY_NOT_ACTIVE");
    let listed = listed.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (listed[0].enabled, listed[0].state),
        (false, MergeBlockState::Off as i32)
    );
    assert!(fake.requests().is_empty());
}

/// A server with no GitHub App cannot change a GitHub rule: `APP_MISSING`, and the rule stays
/// off.
#[sqlx::test(migrations = false)]
async fn set_merge_block_needs_the_github_app(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let linked = linked(&app, "github").await;
    let pipeline = Some(linked.pipeline.as_str());

    // when
    let refused = app
        .set_merge_block(linked.owner, &linked.project, pipeline, true)
        .await;
    let listed = app.list_merge_blocks(linked.owner, &linked.project).await;

    // then
    assert_reason(refused, "APP_MISSING");
    assert_eq!(listed.unwrap()[0].state, MergeBlockState::Off as i32);
}
