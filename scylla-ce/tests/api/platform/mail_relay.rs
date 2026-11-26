//! The tests of the sends through the relay of the mail settings: a test mail gives the answer of
//! the relay, a relay that refuses keeps the mails until a reset, and a new start reads the relay.
//! The relay is a local SMTP relay that sends no mail.

use scylla_core::api::ServeOptions;
use scylla_testkit::{local_relay, Logs, SmtpRelay, TestApp};
use sqlx::PgPool;

use super::mail_settings::{failed_mail, logged, reset_mail, with_auth, EMAIL, PASSWORD, USER};
use crate::app;

/// A test mail goes through the relay at once, and gives its answer and the `Message-ID`.
#[sqlx::test(migrations = false)]
async fn send_test_mail_returns_the_answer_of_the_relay(pool: PgPool) {
    // given
    let logs = Logs::json();
    let app = app(pool).await;
    let admin = app.admin().await;
    let relay = SmtpRelay::start().await;
    app.update_mail_settings(admin, local_relay(relay.port))
        .await
        .unwrap();

    // when
    logs.rearm();
    let answer = app.send_test_mail(admin, "Ada@Example.com").await.unwrap();

    // then
    assert!(answer.accepted, "{answer:?}");
    assert_eq!(answer.host, "127.0.0.1");
    assert_eq!(answer.reply, "250 2.0.0 Ok: queued as Q1");
    assert!(answer.message_id.ends_with("@relay.test>"), "{answer:?}");
    assert_eq!(answer.error, "");
    let mails = relay.wait(1).await;
    assert_eq!(mails[0].to, ["ada@example.com"]);
    assert!(mails[0].data.contains("Subject: Test mail from Scylla"));
    let text = logged(&logs, "mail.tested").await;
    assert!(text.contains("\"to\":\"a***@example.com\""), "{text}");
    assert!(!text.contains("ada@example.com"), "{text}");
}

/// With the default mailer of the server, a test mail goes to it.
#[sqlx::test(migrations = false)]
async fn send_test_mail_uses_the_default_mailer_with_no_relay(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;

    // when
    let answer = app.send_test_mail(admin, EMAIL).await.unwrap();

    // then
    assert!(answer.accepted, "{answer:?}");
    assert_eq!(answer.host, "mailbox.test");
    let mails = app.mails.to(EMAIL).await;
    assert_eq!(mails[0].subject, "Test mail from Scylla");
}

/// A relay that refuses the connection gives its error in the answer of a test mail, and the
/// outbox keeps each mail. A reset gives the default mailer back, which sends the kept mail.
#[sqlx::test(migrations = false)]
async fn a_bad_relay_keeps_the_mails_until_a_reset(pool: PgPool) {
    // given
    let logs = Logs::json();
    let app = app(pool).await;
    let admin = app.admin().await;
    app.user_with(EMAIL, None).await;
    let closed = SmtpRelay::closed_port().await;
    app.update_mail_settings(admin, local_relay(closed))
        .await
        .unwrap();

    // when
    logs.rearm();
    let answer = app.send_test_mail(admin, "ada@example.com").await.unwrap();
    reset_mail(&app).await;
    let state = failed_mail(&app).await;
    let settings = app.reset_mail_settings(admin).await.unwrap();
    sqlx::query!("UPDATE mail_outbox SET next_attempt_at = now() WHERE state = 'pending'")
        .execute(&app.pool)
        .await
        .unwrap();
    let mails = app.mails.wait(EMAIL, 1).await;

    // then
    assert!(!answer.accepted, "{answer:?}");
    assert_eq!(answer.host, "127.0.0.1");
    assert!(answer.error.contains("SMTP"), "{answer:?}");
    assert_eq!(answer.reply, "");
    assert_eq!(state, "pending");
    assert_eq!(settings.relay, None);
    assert!(
        mails[0].subject.contains("password"),
        "{}",
        mails[0].subject
    );
    let text = logged(&logs, "mail.failed").await;
    assert!(text.contains("\"relay\":\"127.0.0.1\""), "{text}");
    let text = logged(&logs, "mail.settings_reset").await;
    assert!(text.contains("\"default_host\":\"mailbox.test\""), "{text}");
}

/// A server that starts again reads the relay and opens its password with the same key. With
/// another key, it cannot open the password, and each send fails with the reason.
#[sqlx::test(migrations = false)]
async fn a_new_start_reads_the_relay_of_the_settings(pool: PgPool) {
    // given
    let first = app(pool.clone()).await;
    let admin = first.admin().await;
    let relay = SmtpRelay::start().await;
    first
        .update_mail_settings(admin, with_auth(relay.port))
        .await
        .unwrap();
    let same_key = ServeOptions {
        sealer: first.sealer.clone(),
        ..ServeOptions::default()
    };

    // when
    let again = TestApp::spawn_with(pool.clone(), scylla_ce::edition(), same_key).await;
    let answer = again.send_test_mail(admin, EMAIL).await.unwrap();
    let other = TestApp::spawn(pool, scylla_ce::edition()).await;
    let refused = other.send_test_mail(admin, EMAIL).await.unwrap();

    // then
    assert!(answer.accepted, "{answer:?}");
    let mails = relay.wait(1).await;
    assert_eq!(mails[0].auth, Some((USER.to_owned(), PASSWORD.to_owned())));
    assert!(!refused.accepted, "{refused:?}");
    assert!(refused.error.contains("cannot be used"), "{refused:?}");
}
