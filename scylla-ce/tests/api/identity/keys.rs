//! The tests of the names of the keys of an App, and of their last use.

use scylla_domain::tenancy::workspaces::Workspace;
use scylla_testkit::{new_key, text};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// A key gets the name that the join or the add sends. An empty name gives a key with no name.
#[sqlx::test(migrations = false)]
async fn a_key_gets_the_name_of_its_join(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();

    // when
    let key = app.join_app_named(&created.join_code, "  build-01 ").await;
    let key = key.unwrap();
    let spare = app.add_app_key(owner, &key.app, &new_key(), "").await;

    // then
    let keys = app.get_app(owner, &key.app).await.unwrap().keys;
    assert_eq!(keys[0].label, "build-01");
    assert_eq!(keys[1].label, "");
    assert_eq!(text(&spare.unwrap().id), text(&keys[1].id));
}

/// A name with a line break is refused at the join, and the code stays open.
#[sqlx::test(migrations = false)]
async fn a_join_checks_the_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();

    // when
    let refused = app.join_app_named(&created.join_code, "build\n01").await;

    // then
    assert_invalid_field(&refused.unwrap_err(), "label");
    assert!(app.join_app(&created.join_code).await.is_ok());
}

/// An exchange records the last use of the key and the last sight of the App.
#[sqlx::test(migrations = false)]
async fn an_exchange_records_the_last_use(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let key = app.join_app(&created.join_code).await.unwrap();
    let before = app.get_app(owner, &key.app).await.unwrap();

    // when
    app.exchange(key.assertion(60)).await.unwrap();

    // then
    assert!(before.keys[0].last_used_at.is_none());
    assert!(before.app.unwrap().last_seen_at.is_none());
    let after = app.get_app(owner, &key.app).await.unwrap();
    assert!(after.keys[0].last_used_at.is_some());
    assert!(after.app.unwrap().last_seen_at.is_some());
}

/// An admin of the workspace renames a key. The new name shows in the reads.
#[sqlx::test(migrations = false)]
async fn rename_app_key_changes_the_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, true).await;

    // when
    let renamed = app.rename_app_key(owner, &key.app, &key.key_id, " laptop ");

    // then
    assert_eq!(renamed.await.unwrap().label, "laptop");
    let keys = app.get_app(owner, &key.app).await.unwrap().keys;
    assert_eq!(keys[0].label, "laptop");
}

/// A rename needs a name, an admin, and a key of the App.
#[sqlx::test(migrations = false)]
async fn rename_app_key_is_checked(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let (key, _) = app.app_in(owner, workspace, true).await;
    let (other, _) = app.app_in(owner, workspace, true).await;

    // when
    let empty = app.rename_app_key(owner, &key.app, &key.key_id, " ").await;
    let by_member = app.rename_app_key(member, &key.app, &key.key_id, "x").await;
    let unknown = app.rename_app_key(owner, &key.app, UNKNOWN_ID, "x").await;
    let elsewhere = app
        .rename_app_key(owner, &key.app, &other.key_id, "x")
        .await;

    // then
    assert_invalid_field(&empty.unwrap_err(), "label");
    assert_code(by_member, Code::PermissionDenied);
    assert_code(unknown, Code::NotFound);
    assert_code(elsewhere, Code::NotFound);
}
