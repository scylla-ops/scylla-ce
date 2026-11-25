//! The tests of the opt-in of the Git events of an editor pipeline in a project of files: only an
//! admin of the workspace gives it, and only then do the push, pull request and tag triggers of
//! the pipeline start runs.

use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::pipelines::v1::create_trigger_request::Kind;
use scylla_proto::pipelines::v1::Push;
use scylla_testkit::{text, FakeProvider};
use sqlx::PgPool;
use tonic::Code;

use super::events::{deliver, push};
use super::{
    app_with, assert_reason, files_project, jobs_of, BUILD, BUILD_PATH, FEATURE_SHA, MAIN_SHA,
    NEXT_SHA,
};
use crate::assert_code;

/// Returns the rule of a push to a branch `release/*`.
fn release() -> Kind {
    Kind::Push(Push {
        branches: vec!["release/*".to_owned()],
        exclude_branches: Vec::new(),
        skip_ci: false,
    })
}

/// Only an admin of the workspace sets the opt-in: a member and an admin of the project get
/// `PERMISSION_DENIED`. The opt-in keeps its actor, a stop removes it, and a file pipeline gives
/// `PIPELINE_READ_ONLY`.
#[sqlx::test(migrations = false)]
async fn set_pipeline_git_events_needs_an_admin_of_the_workspace(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, false).await;
    let (owner, workspace) = (files.linked.owner, files.linked.workspace);
    let editor = files.linked.editor.as_str();
    let (member, admin, project_admin) = (app.user().await, app.user().await, app.user().await);
    let project = ProjectId::parse(&files.linked.project).unwrap();
    app.grant(&[
        Workspace::MEMBER.of(workspace, member),
        Workspace::ADMIN.of(workspace, admin),
        Project::ADMIN.of(project, project_admin),
    ])
    .await;

    // when
    let by_member = app.set_pipeline_git_events(member, editor, true).await;
    let by_project_admin = app.set_pipeline_git_events(project_admin, editor, true);
    let by_project_admin = by_project_admin.await;
    let allowed = app
        .set_pipeline_git_events(admin, editor, true)
        .await
        .unwrap();
    let again = app
        .set_pipeline_git_events(owner, editor, true)
        .await
        .unwrap();
    let stopped = app
        .set_pipeline_git_events(owner, editor, false)
        .await
        .unwrap();
    let file = app.set_pipeline_git_events(owner, &files.build, true).await;

    // then
    assert_code(by_member, Code::PermissionDenied);
    assert_code(by_project_admin, Code::PermissionDenied);
    assert!(allowed.git_events_allowed_at.is_some());
    assert_eq!(text(&allowed.git_events_allowed_by), admin.to_string());
    assert_eq!(again.git_events_allowed_at, allowed.git_events_allowed_at);
    assert_eq!(again.git_events_allowed_by, allowed.git_events_allowed_by);
    assert_eq!(stopped.version, allowed.version);
    assert!(stopped.git_events_allowed_at.is_none() && stopped.git_events_allowed_by.is_none());
    assert_reason(file, "PIPELINE_READ_ONLY");
}

/// In a project of files, a Git trigger of an editor pipeline needs the opt-in
/// (`GIT_EVENTS_NOT_ALLOWED`). With it, a push starts the editor pipeline and the file pipeline.
/// After a stop, a push starts only the file pipeline, and the trigger stays.
#[sqlx::test(migrations = false)]
async fn a_push_starts_an_editor_pipeline_of_a_project_of_files_with_the_opt_in(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    fake.github_files("heads/release/2", FEATURE_SHA, &[(BUILD_PATH, BUILD)]);
    fake.github_files("heads/release/3", NEXT_SHA, &[(BUILD_PATH, BUILD)]);
    let app = app_with(pool, &fake).await;
    let files = files_project(&app, false).await;
    let owner = files.linked.owner;
    let editor = files.linked.editor.as_str();

    // when
    let refused = app
        .create_git_trigger(owner, editor, "push", release())
        .await;
    app.set_pipeline_git_events(owner, editor, true)
        .await
        .unwrap();
    let trigger = app
        .create_git_trigger(owner, editor, "push", release())
        .await;
    let allowed = deliver(
        &app,
        "push",
        "d-1",
        &push("refs/heads/release/2", FEATURE_SHA),
    )
    .await;
    let editor_runs = jobs_of(&app, owner, editor).await.len();
    app.set_pipeline_git_events(owner, editor, false)
        .await
        .unwrap();
    let stopped = deliver(&app, "push", "d-2", &push("refs/heads/release/3", NEXT_SHA)).await;

    // then
    assert_reason(refused, "GIT_EVENTS_NOT_ALLOWED");
    assert!(trigger.is_ok());
    assert_eq!((allowed.error.as_str(), stopped.error.as_str()), ("", ""));
    assert_eq!(editor_runs, 1);
    assert_eq!(jobs_of(&app, owner, editor).await.len(), 1);
    assert_eq!(jobs_of(&app, owner, &files.build).await.len(), 2);
    let listed = app.list_pipeline_triggers(owner, editor).await.unwrap();
    assert_eq!(listed.triggers.len(), 1);
}
