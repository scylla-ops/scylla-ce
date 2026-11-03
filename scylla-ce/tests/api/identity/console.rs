//! The tests of the console of the users: list, read, create, change, disable, delete, reset
//! and sessions of a user, by an instance admin.

use std::time::Duration;

use scylla_proto::accounts::v1::UpdateUserRequest;
use scylla_proto::common::v1::PageRequest;
use scylla_testkit::{id, text};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::{assert_refused, PASSWORD};
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// The list filters on the email, the handle and the display name, in any case, and gives its
/// users one page at a time.
#[sqlx::test(migrations = false)]
async fn list_users_filters_and_pages(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    app.create_user(admin, "amy@corp.example", "amy", PASSWORD)
        .await
        .unwrap();
    app.create_user(admin, "bo@corp.example", "builder", PASSWORD)
        .await
        .unwrap();
    let one = PageRequest {
        size: 1,
        token: String::new(),
    };

    // when
    let by_email = app.list_users(admin, "CORP", PageRequest::default()).await;
    let by_handle = app.list_users(admin, "build", PageRequest::default()).await;
    let by_name = app
        .list_users(admin, "created", PageRequest::default())
        .await;
    let first = app.list_users(admin, "corp", one.clone()).await.unwrap();
    let next = PageRequest {
        token: first.page.clone().unwrap().next_token,
        ..one
    };
    let second = app.list_users(admin, "corp", next).await.unwrap();

    // then
    assert_eq!(by_email.unwrap().users.len(), 2);
    assert_eq!(by_handle.unwrap().users[0].email, "bo@corp.example");
    assert_eq!(by_name.unwrap().users.len(), 2);
    assert_eq!(first.users[0].handle, "amy");
    assert_eq!(second.users[0].handle, "builder");
    assert!(second.page.unwrap().next_token.is_empty());
}

/// A new user is active with a verified email, and signs in with its email or its handle. A
/// used email, a used handle and a short password are refused.
#[sqlx::test(migrations = false)]
async fn create_user_makes_an_active_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    let created = app
        .create_user(admin, "cy@example.com", "cy", PASSWORD)
        .await;
    let same_email = app.create_user(admin, "cy@example.com", "", PASSWORD).await;
    let same_handle = app
        .create_user(admin, "cy2@example.com", "cy", PASSWORD)
        .await;
    let short = app
        .create_user(admin, "cy3@example.com", "", "7 chars")
        .await;

    // then
    let created = created.unwrap();
    assert_eq!(
        (created.status.as_str(), created.handle.as_str()),
        ("active", "cy")
    );
    assert!(app.sign_in("cy", PASSWORD).await.is_ok());
    let read = app.get_user(admin, text(&created.id)).await.unwrap();
    assert_eq!(read.email, "cy@example.com");
    assert_refused(same_email, "EMAIL_IN_USE");
    assert_refused(same_handle, "HANDLE_IN_USE");
    assert_invalid_field(&short.unwrap_err(), "password");
    assert_code(app.get_user(admin, UNKNOWN_ID).await, Code::NotFound);
}

/// An admin changes the display name, the handle and the email of a user. The new email is
/// verified, and an email of another user is refused.
#[sqlx::test(migrations = false)]
async fn update_user_changes_the_profile_and_the_email(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let user = app.user_with("dee@example.com", Some(PASSWORD)).await;
    app.user_with("eli@example.com", None).await;
    let request = |email: &str| UpdateUserRequest {
        user_id: Some(id(user.to_string())),
        display_name: Some("Dee D.".to_owned()),
        handle: Some("dee".to_owned()),
        email: Some(email.to_owned()),
    };

    // when
    let changed = app.update_user(admin, request("dee@new.example")).await;
    let taken = app.update_user(admin, request("eli@example.com")).await;
    let unknown = UpdateUserRequest {
        user_id: Some(id(UNKNOWN_ID)),
        ..Default::default()
    };
    let unknown = app.update_user(admin, unknown).await;

    // then
    let changed = changed.unwrap();
    assert_eq!(
        (
            changed.display_name.as_str(),
            changed.handle.as_str(),
            changed.email.as_str()
        ),
        ("Dee D.", "dee", "dee@new.example")
    );
    assert!(app.sign_in("dee@new.example", PASSWORD).await.is_ok());
    assert_code(
        app.sign_in("dee@example.com", PASSWORD).await,
        Code::Unauthenticated,
    );
    assert_refused(taken, "EMAIL_IN_USE");
    assert_code(unknown, Code::NotFound);
}

/// A new email from an admin stops each open link of the user: a reset link that went to the
/// old email does not work after the change.
#[sqlx::test(migrations = false)]
async fn update_user_stops_the_links_of_the_old_email(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let old = "joy@example.com";
    let user = app.user_with(old, Some(PASSWORD)).await;
    app.request_reset(old).await.unwrap();
    app.mails.wait(old, 1).await;
    let reset = app.mails.token(old, "reset-password");
    let request = UpdateUserRequest {
        user_id: Some(id(user.to_string())),
        email: Some("joy.new@example.com".to_owned()),
        ..Default::default()
    };

    // when
    app.update_user(admin, request).await.unwrap();
    let after = app.reset_password(&reset, "an attacker password").await;

    // then
    assert_invalid_field(&after.unwrap_err(), "token");
    assert!(app.sign_in("joy.new@example.com", PASSWORD).await.is_ok());
}

/// A disabled user loses each session and cannot sign in, until an admin activates it again.
/// An admin cannot change its own status, and a status must be active or disabled.
#[sqlx::test(migrations = false)]
async fn set_user_status_disables_and_closes_the_sessions(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let email = "fox@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await.to_string();
    let session = app.sign_in(email, PASSWORD).await.unwrap();

    // when
    let disabled = app.set_user_status(admin, &user, "disabled").await.unwrap();
    let refused = app.sign_in(email, PASSWORD).await;
    app.set_user_status(admin, &user, "active").await.unwrap();
    let own = app
        .set_user_status(admin, &admin.to_string(), "disabled")
        .await;
    let unknown_status = app.set_user_status(admin, &user, "pending").await;

    // then
    assert_eq!(disabled.status, "disabled");
    assert_code(
        app.refresh(&session.refresh_token).await,
        Code::Unauthenticated,
    );
    assert_code(refused, Code::Unauthenticated);
    assert!(app.sign_in(email, PASSWORD).await.is_ok());
    assert_refused(own, "SELF_NOT_ALLOWED");
    assert_invalid_field(&unknown_status.unwrap_err(), "status");
}

/// An admin deletes a user with the rules of `DeleteAccount`, but not its own account.
#[sqlx::test(migrations = false)]
async fn delete_user_follows_the_rules_of_a_deletion(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let (empty, full) = (app.user().await, app.user().await);
    app.create_project(full, "kept").await.unwrap();

    // when
    let deleted = app.delete_user(admin, &empty.to_string()).await;
    let own = app.delete_user(admin, &admin.to_string()).await;
    let last_owner = app.delete_user(admin, &full.to_string()).await;
    let unknown = app.delete_user(admin, UNKNOWN_ID).await;

    // then
    assert!(deleted.is_ok(), "{deleted:?}");
    assert_code(
        app.get_user(admin, &empty.to_string()).await,
        Code::NotFound,
    );
    assert_refused(own, "SELF_NOT_ALLOWED");
    assert_refused(last_owner, "LAST_OWNER");
    assert_code(unknown, Code::NotFound);
}

/// An admin sends a reset link after its answer, at most one each minute for one user.
#[sqlx::test(migrations = false)]
async fn send_password_reset_sends_one_link(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let email = "gil@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await.to_string();

    // when
    app.send_password_reset(admin, &user).await.unwrap();
    app.mails.wait(email, 1).await;
    app.send_password_reset(admin, &user).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let unknown = app.send_password_reset(admin, UNKNOWN_ID).await;

    // then
    assert_eq!(app.mails.to(email).len(), 1);
    let link = app.mails.token(email, "reset-password");
    assert!(app.reset_password(&link, "a brand new one").await.is_ok());
    assert_code(unknown, Code::NotFound);
}

/// An admin lists the sessions of a user, with their IP address, and closes them all.
#[sqlx::test(migrations = false)]
async fn an_admin_lists_and_closes_the_sessions_of_a_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let email = "hu@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await.to_string();
    let first = app.sign_in(email, PASSWORD).await.unwrap();
    app.sign_in(email, PASSWORD).await.unwrap();

    // when
    let before = app.list_user_sessions(admin, &user).await.unwrap();
    app.revoke_user_sessions(admin, &user).await.unwrap();
    let after = app.list_user_sessions(admin, &user).await.unwrap();
    let unknown = app.list_user_sessions(admin, UNKNOWN_ID).await;

    // then
    assert_eq!(before.len(), 2);
    assert!(before
        .iter()
        .all(|session| session.ip_address == "127.0.0.1" && !session.current));
    assert!(after.is_empty());
    assert_code(
        app.get_me(&first.token.unwrap().value).await,
        Code::Unauthenticated,
    );
    assert_code(unknown, Code::NotFound);
}
