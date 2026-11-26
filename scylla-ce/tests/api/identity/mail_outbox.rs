//! The tests of the mail outbox: a change puts its mail in the outbox, and one task sends it. The
//! task sends it again after a transient error, drops it after a permanent error or after 24
//! hours, and clears its content when it is sent or dropped.

use std::time::Duration;

use scylla_core::SignUpMode;
use scylla_testkit::{Logs, TestApp};
use sqlx::PgPool;

use crate::identity::accounts::{app_in, PASSWORD};

/// The longest time that a test waits for the outbox or for a log line.
const WAIT: Duration = Duration::from_secs(5);

/// The address of the tests.
const EMAIL: &str = "olga@example.com";

/// A row of the outbox, as a test reads it.
#[derive(Debug)]
struct Row {
    /// The kind of the mail.
    kind: String,
    /// The masked recipient.
    recipient: String,
    /// The state.
    state: String,
    /// The sends that ended.
    attempts: i32,
    /// `true` if the row keeps the content of the mail.
    has_content: bool,
}

/// Reads the only row of the outbox when `done` accepts it.
async fn row_when(app: &TestApp, done: impl Fn(&Row) -> bool) -> Row {
    let start = tokio::time::Instant::now();
    loop {
        let row = sqlx::query_as!(
            Row,
            r#"SELECT kind, recipient, state, attempts, content IS NOT NULL AS "has_content!"
               FROM mail_outbox"#
        )
        .fetch_one(&app.pool)
        .await
        .unwrap();
        if done(&row) {
            return row;
        }
        assert!(start.elapsed() < WAIT, "{row:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Makes each mail that waits due at once, as if its wait ended.
async fn make_due(app: &TestApp) {
    sqlx::query!("UPDATE mail_outbox SET next_attempt_at = now() WHERE state = 'pending'")
        .execute(&app.pool)
        .await
        .unwrap();
}

/// Waits until the logs have a line of an event, and returns the logs.
async fn logged(logs: &Logs, event: &str) -> String {
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

/// A sign-up puts the link in the outbox, and the task sends it. The sent mail keeps no content:
/// only its kind, its masked recipient, its state and its attempts. Its log line has the
/// `Message-ID` and the answer of the mail server, with the recipient masked.
#[sqlx::test(migrations = false)]
async fn sign_up_sends_the_link_from_the_outbox_and_clears_it(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let logs = Logs::json();

    // when
    app.sign_up(EMAIL, PASSWORD, "").await.unwrap();
    let mails = app.mails.wait(EMAIL, 1).await;

    // then
    let row = row_when(&app, |row| row.state == "sent").await;
    assert_eq!(row.kind, "verify-email");
    assert_eq!(row.recipient, "o***@example.com");
    assert_eq!(row.attempts, 1);
    assert!(!row.has_content);
    assert!(mails[0].body.contains("/verify-email#token="));
    let text = logged(&logs, "mail.sent").await;
    assert!(text.contains("\"attempts\":1"), "{text}");
    assert!(
        text.contains("\"message_id\":\"<1@mailbox.test>\""),
        "{text}"
    );
    let reply = "250 2.0.0 Ok: queued as 1 for <o***@example.com>";
    assert!(text.contains(&format!("\"reply\":\"{reply}\"")), "{text}");
    assert!(text.contains("\"relay\":\"mailbox.test\""), "{text}");
    assert!(!text.contains(EMAIL), "{text}");
}

/// A transient error keeps the mail in the outbox, and the task sends it again after its wait.
#[sqlx::test(migrations = false)]
async fn sign_up_sends_the_link_again_after_a_transient_error(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    app.mails.fail_next(1);

    // when
    app.sign_up(EMAIL, PASSWORD, "").await.unwrap();
    let failed = row_when(&app, |row| row.attempts == 1).await;
    make_due(&app).await;
    let mails = app.mails.wait(EMAIL, 1).await;

    // then
    assert_eq!(failed.state, "pending");
    assert!(failed.has_content);
    let sent = row_when(&app, |row| row.state == "sent").await;
    assert_eq!(sent.attempts, 2);
    assert!(!sent.has_content);
    assert_eq!(mails.len(), 1);
}

/// A permanent error drops the mail at once, clears its content, and logs `mail.dropped` with
/// the masked recipient only.
#[sqlx::test(migrations = false)]
async fn sign_up_drops_the_link_after_a_permanent_error(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    app.mails.refuse_sends();
    let logs = Logs::json();

    // when
    app.sign_up(EMAIL, PASSWORD, "").await.unwrap();
    let row = row_when(&app, |row| row.state == "dropped").await;

    // then
    assert_eq!(row.attempts, 1);
    assert!(!row.has_content);
    assert!(app.mails.to(EMAIL).await.is_empty());
    let text = logged(&logs, "mail.dropped").await;
    assert!(text.contains("\"to\":\"o***@example.com\""), "{text}");
    assert!(text.contains("\"relay\":\"mailbox.test\""), "{text}");
    assert!(!text.contains(EMAIL) && !text.contains("#token="), "{text}");
}

/// A mail that transient errors stop for 24 hours is dropped, with no content.
#[sqlx::test(migrations = false)]
async fn sign_up_drops_the_link_after_24_hours_of_transient_errors(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    app.mails.fail_sends();
    app.sign_up(EMAIL, PASSWORD, "").await.unwrap();
    row_when(&app, |row| row.attempts == 1).await;

    // when
    sqlx::query!("UPDATE mail_outbox SET created_at = now() - interval '25 hours'")
        .execute(&app.pool)
        .await
        .unwrap();
    make_due(&app).await;
    let row = row_when(&app, |row| row.state == "dropped").await;

    // then
    assert_eq!(row.attempts, 2);
    assert!(!row.has_content);
}
