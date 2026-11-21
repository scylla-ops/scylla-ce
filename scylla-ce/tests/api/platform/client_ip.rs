//! The tests of the client IP address in the log of each call: the full address, its network or
//! no address, as the settings tell. The sessions keep the full address.

use scylla_core::api::ServeOptions;
use scylla_core::infra::logs::LogClientIp;
use scylla_testkit::{Logs, TestApp};
use sqlx::PgPool;

use crate::identity::accounts::PASSWORD;

/// The email of the user of the tests.
const EMAIL: &str = "ip@example.com";

/// Starts the Community edition with a rule of the client IP address, signs in, and returns the
/// log lines of the sign-in and the IP address of the session.
async fn sign_in(pool: PgPool, client_ip: LogClientIp) -> (String, String) {
    // Start the server with the rule, and sign in.
    let options = ServeOptions {
        log_client_ip: client_ip,
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    let user = app.user_with(EMAIL, Some(PASSWORD)).await;
    let logs = Logs::json();
    app.sign_in(EMAIL, PASSWORD).await.unwrap();

    // Read the lines of the call, and the session as an admin sees it.
    let text = logs.text();
    assert!(text.contains("AccountService/SignIn"), "{text}");
    let admin = app.admin().await;
    let sessions = app.list_user_sessions(admin, &user.to_string()).await;
    let ip = sessions.unwrap().remove(0).ip_address;
    (text, ip)
}

/// The default rule logs the network of the address: the first 24 bits of an IPv4 address.
#[sqlx::test(migrations = false)]
async fn the_log_keeps_the_network_of_the_address_by_default(pool: PgPool) {
    // given
    let rule = ServeOptions::default().log_client_ip;

    // when
    let (text, session) = sign_in(pool, rule).await;

    // then
    assert!(text.contains("\"ip\":\"127.0.0.0\""), "{text}");
    assert!(!text.contains("127.0.0.1"), "{text}");
    assert_eq!(session, "127.0.0.1");
}

/// The full rule logs the full address.
#[sqlx::test(migrations = false)]
async fn the_log_keeps_the_full_address_when_full(pool: PgPool) {
    // given
    let rule = LogClientIp::Full;

    // when
    let (text, session) = sign_in(pool, rule).await;

    // then
    assert!(text.contains("\"ip\":\"127.0.0.1\""), "{text}");
    assert_eq!(session, "127.0.0.1");
}

/// The off rule logs no address.
#[sqlx::test(migrations = false)]
async fn the_log_keeps_no_address_when_off(pool: PgPool) {
    // given
    let rule = LogClientIp::Off;

    // when
    let (text, session) = sign_in(pool, rule).await;

    // then
    assert!(text.contains("\"ip\":null"), "{text}");
    assert!(!text.contains("127.0.0."), "{text}");
    assert_eq!(session, "127.0.0.1");
}
