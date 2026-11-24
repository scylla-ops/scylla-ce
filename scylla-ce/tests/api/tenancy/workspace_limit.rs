//! The tests of the limit of the workspaces that one user creates.

use scylla_testkit::text;
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code};

/// The default limit is one workspace, and the personal workspace does not count.
#[sqlx::test(migrations = false)]
async fn create_workspace_keeps_the_limit(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);

    // when
    let settings = app.get_instance_settings(admin).await.unwrap();
    let first = app.create_workspace_as_is(user, "first", None).await;
    let second = app.create_workspace_as_is(user, "second", None).await;
    let other = app.user().await;
    let of_another = app.create_workspace_as_is(other, "another", None).await;

    // then
    assert_eq!(settings.workspace_limit, 1);
    assert!(first.is_ok());
    assert_refused(second, "WORKSPACE_LIMIT");
    assert!(of_another.is_ok());
}

/// An instance admin has no limit.
#[sqlx::test(migrations = false)]
async fn an_instance_admin_has_no_workspace_limit(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    let first = app.create_workspace_as_is(admin, "first", None).await;
    let second = app.create_workspace_as_is(admin, "second", None).await;

    // then
    assert!(first.is_ok());
    assert!(second.is_ok());
}

/// An instance admin changes the limit, and another user cannot. A limit of zero stops each new
/// workspace of a user. A lower limit removes no workspace.
#[sqlx::test(migrations = false)]
async fn the_instance_settings_change_the_workspace_limit(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);

    // when
    let refused = app.set_workspace_limit(user, 5).await;
    let too_large = app.set_workspace_limit(admin, u32::MAX).await;
    let raised = app.set_workspace_limit(admin, 2).await.unwrap();
    let first = app.create_workspace_as_is(user, "first", None).await;
    let second = app.create_workspace_as_is(user, "second", None).await;
    let third = app.create_workspace_as_is(user, "third", None).await;
    app.set_workspace_limit(admin, 0).await.unwrap();
    let other = app.user().await;
    let at_zero = app.create_workspace_as_is(other, "other", None).await;
    let kept = app.visible_workspaces(user).await.unwrap();

    // then
    assert_code(refused, Code::PermissionDenied);
    assert_code(too_large, Code::InvalidArgument);
    assert_eq!(raised.workspace_limit, 2);
    assert!(first.is_ok() && second.is_ok());
    assert_refused(third, "WORKSPACE_LIMIT");
    assert_refused(at_zero, "WORKSPACE_LIMIT");
    assert!(kept.contains(&"first".to_owned()) && kept.contains(&"second".to_owned()));
}

/// A deleted workspace frees its place in the limit.
#[sqlx::test(migrations = false)]
async fn a_deleted_workspace_frees_its_place(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let first = app
        .create_workspace_as_is(user, "first", None)
        .await
        .unwrap();

    // when
    app.delete_workspace(user, text(&first.id)).await.unwrap();
    let again = app.create_workspace_as_is(user, "again", None).await;

    // then
    assert!(again.is_ok());
}

/// Two creations at the same time cannot pass the limit together.
#[sqlx::test(migrations = false)]
async fn two_creations_at_the_same_time_keep_the_limit(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let (first, second) = tokio::join!(
        app.create_workspace_as_is(user, "first", None),
        app.create_workspace_as_is(user, "second", None),
    );

    // then
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
}
