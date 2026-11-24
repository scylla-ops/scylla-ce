//! The tests of the instance admins: only they pass the routes of the console, they give and
//! take back the right, and the last active admin stays.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::admins::UserAdminHolder;
use scylla_domain::platform::Platform;
use scylla_proto::accounts::v1::UpdateUserRequest;
use scylla_proto::common::v1::PageRequest;
use scylla_testkit::{id, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code, UNKNOWN_ID};

/// Returns the code of a call that must fail.
fn code<T: std::fmt::Debug>(result: Result<T, Status>) -> Code {
    result.expect_err("the call must fail").code()
}

/// A user that is not an instance admin gets `PERMISSION_DENIED` on each route of the console,
/// and an admin passes with no role. `GetMe` tells which one is an admin.
#[sqlx::test(migrations = false)]
async fn only_an_instance_admin_passes_the_console(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let target = user.to_string();

    // when
    let update = UpdateUserRequest {
        user_id: Some(id(&target)),
        display_name: Some("Changed".to_owned()),
        ..Default::default()
    };
    let codes = [
        code(app.list_users(user, "", PageRequest::default()).await),
        code(app.get_user(user, &target).await),
        code(
            app.create_user(user, "new@example.com", "", "a long password")
                .await,
        ),
        code(app.update_user(user, update).await),
        code(app.set_user_status(user, &target, "disabled").await),
        code(app.delete_user(user, &admin.to_string()).await),
        code(app.send_password_reset(user, &target).await),
        code(app.list_user_sessions(user, &target).await),
        code(app.revoke_user_sessions(user, &target).await),
        code(app.grant_instance_admin(user, &target).await),
        code(app.revoke_instance_admin(user, &admin.to_string()).await),
    ];
    let by_admin = app.list_users(admin, "", PageRequest::default()).await;

    // then
    assert_eq!(codes, [Code::PermissionDenied; 11]);
    assert_eq!(by_admin.unwrap().users.len(), 2);
    assert!(
        app.can(admin, UserAdminHolder::LIST_USERS.on(Platform::ID))
            .await
    );
    assert!(
        !app.can(user, UserAdminHolder::LIST_USERS.on(Platform::ID))
            .await
    );
    let (admin_me, user_me) = (app.token(admin).await, app.token(user).await);
    assert!(app.get_me(&admin_me).await.unwrap().instance_admin);
    assert!(!app.get_me(&user_me).await.unwrap().instance_admin);
}

/// An admin makes another user an admin, two times with no change, and takes the right back.
/// An unknown user gives `NOT_FOUND`.
#[sqlx::test(migrations = false)]
async fn an_admin_gives_and_takes_back_the_right(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let target = user.to_string();

    // when
    app.grant_instance_admin(admin, &target).await.unwrap();
    app.grant_instance_admin(admin, &target).await.unwrap();
    let as_admin = app.list_users(user, "", PageRequest::default()).await;
    app.revoke_instance_admin(admin, &target).await.unwrap();
    let after = app.list_users(user, "", PageRequest::default()).await;
    let unknown = app.grant_instance_admin(admin, UNKNOWN_ID).await;

    // then
    assert!(as_admin.is_ok(), "{as_admin:?}");
    assert_code(after, Code::PermissionDenied);
    assert_code(unknown, Code::NotFound);
}

/// The last active admin cannot take its own right back. A disabled admin does not count.
#[sqlx::test(migrations = false)]
async fn the_last_active_admin_stays(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (first, second) = (app.admin().await, app.admin().await);
    let (first_id, second_id) = (first.to_string(), second.to_string());

    // when
    app.set_user_status(first, &second_id, "disabled")
        .await
        .unwrap();
    let alone = app.revoke_instance_admin(first, &first_id).await;
    app.set_user_status(first, &second_id, "active")
        .await
        .unwrap();
    let with_another = app.revoke_instance_admin(first, &first_id).await;

    // then
    assert_refused(alone, "LAST_ADMIN");
    assert!(with_another.is_ok(), "{with_another:?}");
    assert_code(
        app.list_users(first, "", PageRequest::default()).await,
        Code::PermissionDenied,
    );
}

/// Makes two new users the only instance admins, and returns them.
async fn only_admins(app: &TestApp) -> (ActorId, ActorId) {
    sqlx::query!("DELETE FROM instance_admins")
        .execute(&app.pool)
        .await
        .unwrap();
    (app.admin().await, app.admin().await)
}

/// Counts the instance admins with an active account.
async fn active_admins(app: &TestApp) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM instance_admins a
           JOIN users u ON u.id = a.user_id WHERE u.status = 'active'"#
    )
    .fetch_one(&app.pool)
    .await
    .unwrap()
}

/// Two admins that disable each other at the same time leave one active admin: the changes
/// lock the admins, then read their states. Ten rounds, so that a race shows.
#[sqlx::test(migrations = false)]
async fn two_admins_cannot_disable_each_other(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let mut active = Vec::new();
    for _ in 0..10 {
        let (first, second) = only_admins(&app).await;
        let (first_id, second_id) = (first.to_string(), second.to_string());
        let _ = tokio::join!(
            app.set_user_status(first, &second_id, "disabled"),
            app.set_user_status(second, &first_id, "disabled"),
        );
        active.push(active_admins(&app).await);
    }

    // then
    assert!(active.iter().all(|&count| count == 1), "{active:?}");
}
