//! The tests of the labels of an App: the labels that the admins give with `SetAppLabels`, and
//! the automatic labels that the hello of an agent gives.

use scylla_domain::identity::apps::AppHolder;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_testkit::text;
use sqlx::PgPool;
use tonic::Code;

use super::agents::connected;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// An admin of the workspace replaces the labels of an App. The reads show them in their order,
/// and an empty list removes them.
#[sqlx::test(migrations = false)]
async fn set_app_labels_replaces_the_labels(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, admin) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::ADMIN.of(workspace, admin)]).await;
    let (key, _) = app.app_in(owner, workspace, true).await;

    // when
    let set = app
        .set_app_labels(admin, &key.app, &["deploy", " gpu "])
        .await;
    let read = app.get_app(owner, &key.app).await.unwrap().app.unwrap();
    let listed = app.list_apps(owner, workspace, Some(true), 10, "").await;
    let cleared = app.set_app_labels(owner, &key.app, &[]).await.unwrap();

    // then
    assert_eq!(set.unwrap().labels, ["deploy", "gpu"]);
    assert_eq!(read.labels, ["deploy", "gpu"]);
    assert_eq!(listed.unwrap().apps[0].labels, ["deploy", "gpu"]);
    assert!(cleared.labels.is_empty());
}

/// A label has the form of a label, an App has at most 16 labels, each one time, and the
/// refusal names the bad value.
#[sqlx::test(migrations = false)]
async fn set_app_labels_checks_the_labels(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, true).await;
    let many: Vec<String> = (0..17).map(|n| format!("l{n}")).collect();
    let many: Vec<&str> = many.iter().map(String::as_str).collect();

    // when
    let bad = app
        .set_app_labels(owner, &key.app, &["deploy", "Prod"])
        .await;
    let twice = app.set_app_labels(owner, &key.app, &["a", "a"]).await;
    let too_many = app.set_app_labels(owner, &key.app, &many).await;
    let long = "a".repeat(33);
    let too_long = app.set_app_labels(owner, &key.app, &[long.as_str()]).await;

    // then
    assert_invalid_field(&bad.unwrap_err(), "labels[1]");
    assert_invalid_field(&twice.unwrap_err(), "labels[1]");
    assert_invalid_field(&too_many.unwrap_err(), "labels");
    assert_invalid_field(&too_long.unwrap_err(), "labels[0]");
    let read = app.get_app(owner, &key.app).await.unwrap().app.unwrap();
    assert!(read.labels.is_empty());
}

/// Only the admins of the workspace set the labels: not a member, not the creator of the App
/// with no admin role, and not the App itself. An unknown App gets the same refusal.
#[sqlx::test(migrations = false)]
async fn set_app_labels_needs_an_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member, creator) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[
        Workspace::MEMBER.of(workspace, member),
        AppHolder::APP_CREATOR.of(workspace, creator),
    ])
    .await;
    let created = app.create_app(creator, workspace, true).await.unwrap();
    let own = text(&created.app.unwrap().id).to_owned();
    let (key, _) = app.app_in(owner, workspace, true).await;

    // when
    let by_member = app.set_app_labels(member, &key.app, &["deploy"]).await;
    let by_creator = app.set_app_labels(creator, &own, &["deploy"]).await;
    let by_itself = app.set_app_labels(key.actor(), &key.app, &["deploy"]).await;
    let unknown = app.set_app_labels(owner, UNKNOWN_ID, &["deploy"]).await;

    // then
    assert_code(by_member, Code::PermissionDenied);
    assert_code(by_creator, Code::PermissionDenied);
    assert_code(by_itself, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
}

/// The hello gives the automatic labels of the agent: its system, its architecture, and
/// `container` with a container runtime. They stay apart from the labels of the admins.
#[sqlx::test(migrations = false)]
async fn open_stream_gives_the_automatic_labels(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (before, _) = app.app_in(user, workspace, true).await;

    // when
    let (key, _sender, _stream) = connected(&app, user).await;
    app.set_app_labels(user, &key.app, &["deploy"])
        .await
        .unwrap();
    let agent = app.get_app(user, &key.app).await.unwrap().app.unwrap();
    let new = app.get_app(user, &before.app).await.unwrap().app.unwrap();

    // then
    assert_eq!(agent.automatic_labels, ["linux", "x64", "container"]);
    assert_eq!(agent.labels, ["deploy"]);
    assert!(new.automatic_labels.is_empty());
}
