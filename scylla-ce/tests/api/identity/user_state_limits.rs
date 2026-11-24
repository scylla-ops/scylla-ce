//! The tests of the limits of the state of a user: the form of a key, the form and the size of a
//! value, the number of keys and the size of all the entries.

use scylla_domain::identity::actors::ActorId;
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::{app, assert_invalid_field};

/// Returns a JSON text of this number of bytes.
fn text_of(bytes: usize) -> String {
    format!("\"{}\"", "a".repeat(bytes - 2))
}

/// Checks that a call gets `FAILED_PRECONDITION` with the reason `USER_STATE_FULL`.
fn assert_full<T: std::fmt::Debug>(result: Result<T, Status>) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo detail");
    assert_eq!(info.reason, "USER_STATE_FULL");
}

/// Writes keys `k0`, `k1` and so on with a small value.
async fn fill(app: &TestApp, user: ActorId, count: usize) {
    for index in 0..count {
        app.put_user_state(user, &format!("k{index}"), "1", None)
            .await
            .unwrap();
    }
}

/// A key has 1 to 128 lower case letters, digits, dots, colons, dashes or underscores.
#[sqlx::test(migrations = false)]
async fn put_user_state_entry_checks_the_key(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let (longest, too_long) = ("a".repeat(128), "a".repeat(129));

    // when
    let mut refused = Vec::new();
    for key in ["", "Upper", "with space", "é", too_long.as_str()] {
        refused.push(app.put_user_state(user, key, "1", None).await);
    }
    let mut accepted = Vec::new();
    for key in ["onboarding:a.b-c_1", longest.as_str()] {
        accepted.push(app.put_user_state(user, key, "1", None).await);
    }

    // then
    for result in refused {
        assert_invalid_field(&result.unwrap_err(), "key");
    }
    for result in accepted {
        assert!(result.is_ok(), "{result:?}");
    }
}

/// A value is a JSON text of at most 16 KiB that `jsonb` can keep.
#[sqlx::test(migrations = false)]
async fn put_user_state_entry_checks_the_value(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let (largest, too_large) = (text_of(16 << 10), text_of((16 << 10) + 1));
    let bad = [
        "",
        "{",
        "undefined",
        "1e400",
        "\"\\u0000\"",
        "[{\"a\":\"\\u0000\"}]",
        "{\"\\u0000\":1}",
        too_large.as_str(),
    ];

    // when
    let mut refused = Vec::new();
    for value in bad {
        refused.push(app.put_user_state(user, "key", value, None).await);
    }
    let null = app.put_user_state(user, "null", "null", None).await;
    let at_limit = app.put_user_state(user, "large", &largest, None).await;

    // then
    for result in refused {
        assert_invalid_field(&result.unwrap_err(), "value_json");
    }
    assert_eq!(null.unwrap().value_json, "null");
    assert_eq!(at_limit.unwrap().value_json, largest);
}

/// A user has at most 32 keys, and a key that has an entry can still change.
#[sqlx::test(migrations = false)]
async fn put_user_state_entry_keeps_at_most_32_keys(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    fill(&app, user, 32).await;

    // when
    let new_key = app.put_user_state(user, "k32", "1", None).await;
    let same_key = app.put_user_state(user, "k0", "2", None).await;

    // then
    assert_full(new_key);
    assert_eq!(same_key.unwrap().version, 2);
    assert_eq!(app.get_user_state(user).await.unwrap().len(), 32);
}

/// The keys and the values of a user hold at most 64 KiB. A write that makes an entry smaller
/// passes, and a refused write changes nothing.
#[sqlx::test(migrations = false)]
async fn put_user_state_entry_keeps_at_most_64_kib(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let full = text_of(16 << 10);
    for key in ["k0", "k1", "k2"] {
        app.put_user_state(user, key, &full, None).await.unwrap();
    }
    let room = (64 << 10) - 3 * (2 + (16 << 10)) - 2;

    // when
    let above = app
        .put_user_state(user, "k3", &text_of(room + 1), None)
        .await;
    let at_limit = app.put_user_state(user, "k3", &text_of(room), None).await;
    let new_key = app.put_user_state(user, "k4", "1", None).await;
    let larger = app
        .put_user_state(user, "k3", &text_of(room + 1), None)
        .await;
    let smaller = app.put_user_state(user, "k3", "1", None).await;
    let after = app.put_user_state(user, "k4", "1", None).await;

    // then
    assert_full(above);
    assert_eq!(at_limit.unwrap().version, 1);
    assert_full(new_key);
    assert_full(larger);
    assert_eq!(smaller.unwrap().version, 2);
    assert!(after.is_ok(), "{after:?}");
}

/// Two writes at one time count the entries one after the other, so the limit stays true.
#[sqlx::test(migrations = false)]
async fn put_user_state_entry_counts_two_writes_at_one_time(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    fill(&app, user, 31).await;

    // when
    let (first, second) = tokio::join!(
        app.put_user_state(user, "first", "1", None),
        app.put_user_state(user, "second", "1", None),
    );

    // then
    assert!(first.is_ok() != second.is_ok(), "{first:?} {second:?}");
    assert_eq!(app.get_user_state(user).await.unwrap().len(), 32);
}
