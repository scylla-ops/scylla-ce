//! The tests of the state of a user: the values, the expected version, the callers and the
//! deletion of an account.

use scylla_proto::accounts::v1::GetUserStateRequest;
use scylla_testkit::with_token;
use sqlx::PgPool;
use tonic::{Code, Request};

use crate::identity::accounts::PASSWORD;
use crate::{app, assert_code};

/// A first write gives version 1, the next write gives version 2, and a read gives the last value.
#[sqlx::test(migrations = false)]
async fn put_user_state_entry_keeps_the_value(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let empty = app.get_user_state(user).await.unwrap();

    // when
    let first = app.put_user_state(user, "theme", "\"dark\"", None).await;
    let second = app
        .put_user_state(user, "theme", "{\"mode\":  \"light\"}", None)
        .await;
    let entries = app.get_user_state(user).await.unwrap();

    // then
    assert!(empty.is_empty());
    assert_eq!(first.unwrap().version, 1);
    assert_eq!(second.unwrap().version, 2);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].key, "theme");
    assert_eq!(entries[0].version, 2);
    assert!(entries[0].updated_at.is_some());
    let value: serde_json::Value = serde_json::from_str(&entries[0].value_json).unwrap();
    assert_eq!(value, serde_json::json!({ "mode": "light" }));
}

/// A user reads only its own entries, and one key of two users has two versions.
#[sqlx::test(migrations = false)]
async fn each_user_has_its_own_state(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (ada, ben) = (app.user().await, app.user().await);
    app.put_user_state(ada, "theme", "1", None).await.unwrap();
    app.put_user_state(ada, "theme", "2", None).await.unwrap();

    // when
    let before = app.get_user_state(ben).await.unwrap();
    let written = app.put_user_state(ben, "theme", "3", None).await.unwrap();
    let of_ada = app.get_user_state(ada).await.unwrap();

    // then
    assert!(before.is_empty());
    assert_eq!(written.version, 1);
    assert_eq!(of_ada[0].value_json, "2");
    assert_eq!(of_ada[0].version, 2);
}

/// Not set: the write passes. 0: the write passes only if the key has no entry. Another number:
/// the write passes only if the entry has this version. A refusal changes nothing.
#[sqlx::test(migrations = false)]
async fn put_user_state_entry_checks_the_expected_version(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let unknown = app.put_user_state(user, "key", "1", Some(1)).await;
    let created = app.put_user_state(user, "key", "2", Some(0)).await;
    let not_new = app.put_user_state(user, "key", "3", Some(0)).await;
    let stale = app.put_user_state(user, "key", "4", Some(5)).await;
    let current = app.put_user_state(user, "key", "5", Some(1)).await;
    let any = app.put_user_state(user, "key", "6", None).await;

    // then
    assert_code(unknown, Code::Aborted);
    assert_eq!(created.unwrap().version, 1);
    assert_code(not_new, Code::Aborted);
    assert_code(stale, Code::Aborted);
    assert_eq!(current.unwrap().version, 2);
    assert_eq!(any.unwrap().version, 3);
}

/// A call with no token gets `UNAUTHENTICATED`.
#[sqlx::test(migrations = false)]
async fn a_call_with_no_token_is_refused(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let result = app
        .user_state()
        .get_user_state(Request::new(GetUserStateRequest {}))
        .await;

    // then
    assert_code(result, Code::Unauthenticated);
}

/// An App has no state.
#[sqlx::test(migrations = false)]
async fn an_app_cannot_call_the_routes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (_key, token) = app.app_in(owner, workspace, false).await;

    // when
    let result = app
        .user_state()
        .get_user_state(with_token(&token, GetUserStateRequest {}))
        .await;

    // then
    assert_code(result, Code::PermissionDenied);
}

/// A token from an exchange of a personal token has no state, and a session has one.
#[sqlx::test(migrations = false)]
async fn an_api_token_cannot_call_the_routes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "state@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    let created = app.create_personal_token(user, "ci", None).await.unwrap();
    let exchanged = app.exchange_api_token(&created.secret).await.unwrap();
    let session = app.sign_in(email, PASSWORD).await.unwrap();
    let signed_in = session.token.unwrap().value;

    // when
    let by_token = app
        .user_state()
        .get_user_state(with_token(&exchanged, GetUserStateRequest {}))
        .await;
    let by_session = app
        .user_state()
        .get_user_state(with_token(&signed_in, GetUserStateRequest {}))
        .await;

    // then
    assert_code(by_token, Code::PermissionDenied);
    assert!(by_session.is_ok(), "{by_session:?}");
}

/// A deletion of the account removes the entries of the user.
#[sqlx::test(migrations = false)]
async fn delete_account_removes_the_state(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "gone@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    app.put_user_state(user, "theme", "1", None).await.unwrap();
    let session = app.sign_in(email, PASSWORD).await.unwrap();
    let token = session.token.unwrap().value;

    // when
    app.delete_account(&token, PASSWORD).await.unwrap();

    // then
    let rows = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM user_state WHERE user_id = $1"#,
        user.to_db()
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(rows, 0);
}
