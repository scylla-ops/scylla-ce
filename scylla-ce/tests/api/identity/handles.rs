//! The tests of the handles: their form and its limits, one user for each handle, and the
//! sign-in with a handle.

use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::{assert_refused, PASSWORD};
use crate::{app, assert_code, assert_invalid_field};

/// A wrong password of the tests.
const WRONG: &str = "not the password";

/// A handle has 2 to 39 characters. Upper case becomes lower case, and an empty handle removes
/// the handle.
#[sqlx::test(migrations = false)]
async fn update_profile_checks_the_length_of_the_handle(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "kim@example.com";
    app.user_with(email, Some(PASSWORD)).await;
    let token = app
        .sign_in(email, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value;
    let longest = "k".repeat(39);

    // when
    let one = app.update_profile(&token, None, Some("k")).await;
    let two = app.update_profile(&token, None, Some("KI")).await.unwrap();
    let max = app
        .update_profile(&token, None, Some(&longest))
        .await
        .unwrap();
    let over = app
        .update_profile(&token, None, Some(&"k".repeat(40)))
        .await;
    let removed = app.update_profile(&token, None, Some("")).await.unwrap();

    // then
    assert_invalid_field(&one.unwrap_err(), "handle");
    assert_eq!(two.handle, "ki");
    assert_eq!(max.handle, longest);
    assert_invalid_field(&over.unwrap_err(), "handle");
    assert_eq!(removed.handle, "");
}

/// A handle starts with a letter or a digit, and has only letters, digits, dots, dashes and
/// underscores: no `@`, so a sign-in tells it from an email.
#[sqlx::test(migrations = false)]
async fn update_profile_checks_the_characters_of_the_handle(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "lea@example.com";
    app.user_with(email, Some(PASSWORD)).await;
    let token = app
        .sign_in(email, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value;

    // when
    let valid = app.update_profile(&token, None, Some("l3a.b_c-d")).await;
    let mut refused = Vec::new();
    for handle in [".lea", "-lea", "le@a", "le a", "lé"] {
        refused.push(app.update_profile(&token, None, Some(handle)).await);
    }

    // then
    assert_eq!(valid.unwrap().handle, "l3a.b_c-d");
    for result in refused {
        assert_invalid_field(&result.unwrap_err(), "handle");
    }
}

/// Two users cannot have the same handle. A change of the display name keeps the handle.
#[sqlx::test(migrations = false)]
async fn a_handle_has_one_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (first, second) = ("max@example.com", "ned@example.com");
    app.user_with(first, Some(PASSWORD)).await;
    app.user_with(second, Some(PASSWORD)).await;
    let first = app
        .sign_in(first, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value;
    let second = app
        .sign_in(second, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value;
    app.update_profile(&first, None, Some("max")).await.unwrap();

    // when
    let taken = app.update_profile(&second, None, Some("MAX")).await;
    let renamed = app
        .update_profile(&first, Some("Max B."), None)
        .await
        .unwrap();

    // then
    assert_refused(taken, "HANDLE_IN_USE");
    assert_eq!(
        (renamed.display_name.as_str(), renamed.handle.as_str()),
        ("Max B.", "max")
    );
}

/// A user signs in with its handle, in any case. An unknown handle, a text that is not a
/// handle, and a wrong password get the same answer, and the lock counts the failures of the
/// handle.
#[sqlx::test(migrations = false)]
async fn sign_in_with_a_handle(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "ora@example.com";
    app.user_with(email, Some(PASSWORD)).await;
    let token = app
        .sign_in(email, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value;
    app.update_profile(&token, None, Some("ora")).await.unwrap();

    // when
    let by_handle = app.sign_in("ORA", PASSWORD).await;
    let wrong = app.sign_in("ora", WRONG).await.unwrap_err();
    let unknown = app.sign_in("nobody", PASSWORD).await.unwrap_err();
    let not_a_handle = app.sign_in("x", PASSWORD).await.unwrap_err();
    for _ in 0..4 {
        app.sign_in("ora", WRONG).await.unwrap_err();
    }
    let locked = app.sign_in("ora", PASSWORD).await;

    // then
    let me = app.get_me(&by_handle.unwrap().token.unwrap().value).await;
    assert_eq!(me.unwrap().email, email);
    assert_eq!(wrong.code(), Code::Unauthenticated);
    for other in [&unknown, &not_a_handle] {
        assert_eq!(
            (other.code(), other.message()),
            (wrong.code(), wrong.message())
        );
    }
    assert_code(locked, Code::Unauthenticated);
}
