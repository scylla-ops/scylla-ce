//! The tests of `MailSettingsService`: an instance admin sets and removes the relay of the
//! instance, with its password sealed and never shown. `mail_relay` tests the sends. The relay is
//! a local SMTP relay that sends no mail.

use std::time::Duration;

use scylla_proto::platform::v1::{MailSecurity, UpdateMailSettingsRequest};
use scylla_testkit::{local_relay, Logs, SmtpRelay, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_code, assert_invalid_field};

/// The longest time that a test waits for the outbox or for a log line.
pub(super) const WAIT: Duration = Duration::from_secs(10);

/// The address of the account that gets the mails of the tests.
pub(super) const EMAIL: &str = "olga@example.com";

/// The user name of the relay of the tests.
pub(super) const USER: &str = "relay-user";

/// The password of the relay of the tests.
pub(super) const PASSWORD: &str = "relay-pass-7Kq";

/// Makes a request for the local relay with a user name and a password.
pub(super) fn with_auth(port: u16) -> UpdateMailSettingsRequest {
    UpdateMailSettingsRequest {
        username: USER.to_owned(),
        password: Some(PASSWORD.to_owned()),
        ..local_relay(port)
    }
}

/// Asks a reset link for the account of `EMAIL`: the server puts the mail in the outbox.
pub(super) async fn reset_mail(app: &TestApp) {
    app.request_reset(EMAIL).await.unwrap();
}

/// Waits until the logs have a line of an event, and returns the logs.
pub(super) async fn logged(logs: &Logs, event: &str) -> String {
    let start = tokio::time::Instant::now();
    loop {
        let text = logs.text();
        if text.contains(&format!("\"event\":\"{event}\"")) {
            return text;
        }
        assert!(start.elapsed() < WAIT, "no {event}:\n{text}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Waits until the only mail of the outbox has failed sends, and returns its state.
pub(super) async fn failed_mail(app: &TestApp) -> String {
    let start = tokio::time::Instant::now();
    loop {
        let row = sqlx::query!("SELECT state, attempts FROM mail_outbox")
            .fetch_optional(&app.pool)
            .await
            .unwrap();
        if let Some(row) = row.filter(|row| row.attempts > 0) {
            return row.state;
        }
        assert!(start.elapsed() < WAIT, "the mail was not sent");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A new instance has no relay, and shows the host of the default mailer of the server.
#[sqlx::test(migrations = false)]
async fn get_mail_settings_has_no_relay_on_a_new_instance(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    let settings = app.get_mail_settings(admin).await.unwrap();

    // then
    assert_eq!(settings.relay, None);
    assert_eq!(settings.default_host, "mailbox.test");
}

/// After a change, the next mail of the outbox goes through the new relay with its user name and
/// password, from the new sender, and the sign-in page shows the new sender. The answer never
/// has the password.
#[sqlx::test(migrations = false)]
async fn update_mail_settings_sends_the_next_mails_through_the_relay(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    app.user_with(EMAIL, None).await;
    let relay = SmtpRelay::start().await;

    // when
    let settings = app
        .update_mail_settings(admin, with_auth(relay.port))
        .await
        .unwrap();
    reset_mail(&app).await;
    let mails = relay.wait(1).await;

    // then
    assert!(!format!("{settings:?}").contains(PASSWORD));
    let saved = settings.relay.unwrap();
    assert_eq!(saved.host, "127.0.0.1");
    assert_eq!(saved.username, USER);
    assert!(saved.password_set);
    assert_eq!(mails[0].to, [EMAIL]);
    assert_eq!(mails[0].from, "no-reply@relay.test");
    assert_eq!(mails[0].auth, Some((USER.to_owned(), PASSWORD.to_owned())));
    assert_eq!(mails[0].hello, "[127.0.0.1]");
    assert!(mails[0].data.contains("From: Scylla <no-reply@relay.test>"));
    assert!(app.mails.to(EMAIL).await.is_empty());
    let methods = app.sign_in_methods().await.unwrap();
    assert_eq!(methods.mail_sender, "no-reply@relay.test");
}

/// A change with no password keeps the saved password, and a relay with no user name has none.
#[sqlx::test(migrations = false)]
async fn update_mail_settings_keeps_the_saved_password(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let relay = SmtpRelay::start().await;
    app.update_mail_settings(admin, with_auth(relay.port))
        .await
        .unwrap();

    // when
    let kept = UpdateMailSettingsRequest {
        password: None,
        from_name: "Scylla Ops".to_owned(),
        ..with_auth(relay.port)
    };
    let kept = app.update_mail_settings(admin, kept).await.unwrap();
    app.send_test_mail(admin, EMAIL).await.unwrap();
    let mails = relay.wait(1).await;
    let none = app
        .update_mail_settings(admin, local_relay(relay.port))
        .await
        .unwrap();

    // then
    assert!(kept.relay.unwrap().password_set);
    assert_eq!(mails[0].auth, Some((USER.to_owned(), PASSWORD.to_owned())));
    assert!(mails[0].data.contains("Scylla Ops"), "{}", mails[0].data);
    assert!(!none.relay.unwrap().password_set);
}

/// A bad value gives `INVALID_ARGUMENT` with its field, and changes nothing.
#[sqlx::test(migrations = false)]
async fn update_mail_settings_refuses_a_bad_value(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let base = with_auth(2525);
    let cases = [
        (
            UpdateMailSettingsRequest {
                host: "smtp relay".to_owned(),
                ..base.clone()
            },
            "host",
        ),
        (
            UpdateMailSettingsRequest {
                port: 0,
                ..base.clone()
            },
            "port",
        ),
        (
            UpdateMailSettingsRequest {
                security: MailSecurity::Unspecified as i32,
                ..base.clone()
            },
            "security",
        ),
        (
            UpdateMailSettingsRequest {
                from_address: "no-reply".to_owned(),
                ..base.clone()
            },
            "from_address",
        ),
        (
            UpdateMailSettingsRequest {
                password: Some("a\nb".to_owned()),
                ..base.clone()
            },
            "password",
        ),
        (
            UpdateMailSettingsRequest {
                password: None,
                ..base.clone()
            },
            "password",
        ),
    ];

    // when
    let mut statuses = Vec::new();
    for (request, _) in &cases {
        let result = app.update_mail_settings(admin, request.clone()).await;
        statuses.push(result.unwrap_err());
    }

    // then
    for (status, (_, field)) in statuses.iter().zip(&cases) {
        assert_invalid_field(status, field);
    }
    let settings = app.get_mail_settings(admin).await.unwrap();
    assert_eq!(settings.relay, None);
}

/// With no key of the secrets, a password gives `SECRET_KEY_MISSING`, and a relay with no
/// password works.
#[sqlx::test(migrations = false)]
async fn update_mail_settings_needs_the_key_for_a_password(pool: PgPool) {
    // given
    let app = TestApp::spawn_without_secret_key(pool, scylla_ce::edition()).await;
    let admin = app.admin().await;

    // when
    let refused = app.update_mail_settings(admin, with_auth(2525)).await;
    let saved = app.update_mail_settings(admin, local_relay(2525)).await;

    // then
    assert_refused(refused, "SECRET_KEY_MISSING");
    assert!(!saved.unwrap().relay.unwrap().password_set);
}

/// The change and the reset log an event with the relay and no secret: no user name and no
/// password.
#[sqlx::test(migrations = false)]
async fn update_mail_settings_logs_no_secret(pool: PgPool) {
    // given
    let logs = Logs::json();
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    logs.rearm();
    app.update_mail_settings(admin, with_auth(2525))
        .await
        .unwrap();
    let text = logged(&logs, "mail.settings_changed").await;

    // then
    assert!(text.contains("\"host\":\"127.0.0.1\""), "{text}");
    assert!(text.contains("\"port\":2525"), "{text}");
    assert!(text.contains("\"auth\":true"), "{text}");
    assert!(!text.contains(PASSWORD) && !text.contains(USER), "{text}");
}

/// A user who is not an instance admin gets `PERMISSION_DENIED` on each route.
#[sqlx::test(migrations = false)]
async fn mail_settings_routes_refuse_a_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let get = app.get_mail_settings(user).await;
    let update = app.update_mail_settings(user, local_relay(2525)).await;
    let reset = app.reset_mail_settings(user).await;
    let test = app.send_test_mail(user, EMAIL).await;

    // then
    assert_code(get, Code::PermissionDenied);
    assert_code(update, Code::PermissionDenied);
    assert_code(reset, Code::PermissionDenied);
    assert_code(test, Code::PermissionDenied);
}
