//! The tests of the GitLab connections: the check of the token of a service account, its seal,
//! its warnings, and its replacement.

use scylla_proto::integrations::v1::{AccountKind, AuthKind, ConnectionState, VcsProvider};
use scylla_testkit::TestApp;
use serde_json::{json, Value};
use sqlx::PgPool;

use super::{app_with_fake, assert_reason};
use crate::assert_invalid_field;

/// The token of the tests.
const TOKEN: &str = "glpat-first-secret";

/// Returns the date in some days, as GitLab gives the expiry of a token.
async fn date_in(app: &TestApp, days: i32) -> Value {
    let date: String =
        sqlx::query_scalar("SELECT to_char(now() + make_interval(days => $1), 'YYYY-MM-DD')")
            .bind(days)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    Value::String(date)
}

/// Opens the sealed token of a connection with the id of the connection.
async fn opened(app: &TestApp, connection: &str) -> String {
    let sealed = app.sealed_token_of(connection).await.expect("a token");
    let id = uuid::Uuid::parse_str(connection).unwrap();
    let sealer = app.sealer.as_ref().expect("a key");
    String::from_utf8(sealer.open_with(&sealed, id.as_bytes()).unwrap()).unwrap()
}

/// A token of a service account makes a connection: its scopes, its expiry, its account and the
/// count of its projects. The token is sealed with the id of the connection, and no answer has
/// it.
#[sqlx::test(migrations = false)]
async fn connect_gitlab_checks_and_seals_the_token(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let projects = [(1, "bank/payments", 40), (2, "bank/ledger", 50)];
    let expiry = date_in(&app, 300).await;
    fake.answer_gitlab_token((42, "scylla-bot", true), &["api"], expiry, &projects);

    // when
    let made = app.connect_gitlab(owner, &workspace, TOKEN).await.unwrap();

    // then
    assert!(made.warnings.is_empty(), "{:?}", made.warnings);
    let connection = made.connection.expect("a connection");
    assert_eq!(connection.provider, VcsProvider::Gitlab as i32);
    assert_eq!(connection.auth_kind, AuthKind::AccessToken as i32);
    assert_eq!(connection.account_kind, AccountKind::ServiceAccount as i32);
    assert_eq!(connection.state, ConnectionState::Active as i32);
    assert_eq!(
        (connection.account.as_str(), connection.external_id.as_str()),
        ("scylla-bot", "42")
    );
    assert_eq!(connection.repository_count, 2);
    assert_eq!(connection.scopes, ["api"]);
    assert!(connection.token_expires_at.is_some());
    assert_eq!(
        connection.settings_url,
        format!("{}/scylla-bot", fake.url())
    );
    assert!(!format!("{connection:?}").contains(TOKEN));
    let id = &connection.id.as_ref().unwrap().value;
    assert_eq!(opened(&app, id).await, TOKEN);
    let read = &fake.requests_to("GET", "/api/v4/personal_access_tokens/self")[0];
    assert_eq!(read.header("private-token"), Some(TOKEN));
    let listed = &fake.requests_to("GET", "/api/v4/projects")[0];
    assert!(listed.query.contains("membership=true&min_access_level=30"));
}

/// A token of a person that expires in 6 days and is Developer on a project connects, with a
/// warning for each risk.
#[sqlx::test(migrations = false)]
async fn connect_gitlab_warns_about_each_risk(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let expiry = date_in(&app, 6).await;
    let projects = [(1, "lea/payments", 30)];
    fake.answer_gitlab_token((7, "lea", false), &["api"], expiry, &projects);

    // when
    let made = app.connect_gitlab(owner, &workspace, TOKEN).await.unwrap();

    // then
    let warnings = [
        "NOT_A_SERVICE_ACCOUNT",
        "TOKEN_EXPIRES_SOON",
        "ROLE_DEVELOPER",
    ];
    assert_eq!(made.warnings, warnings);
    let connection = made.connection.unwrap();
    assert_eq!(connection.account_kind, AccountKind::User as i32);
}

/// GitLab refuses a token: `TOKEN_INVALID`. A token with no scope api: `TOKEN_SCOPE_MISSING`. A
/// blank token is a bad request. An account with a connection: `CONNECTION_EXISTS`.
#[sqlx::test(migrations = false)]
async fn connect_gitlab_refuses_a_bad_token(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let path = "/api/v4/personal_access_tokens/self";
    let refused = json!({ "message": "401 Unauthorized" });
    fake.respond("GET", path, 401, refused, &[]);
    fake.answer_gitlab_token((42, "scylla-bot", true), &["read_api"], Value::Null, &[]);
    fake.answer_gitlab_token((42, "scylla-bot", true), &["api"], Value::Null, &[]);

    // when
    let invalid = app.connect_gitlab(owner, &workspace, TOKEN).await;
    let narrow = app.connect_gitlab(owner, &workspace, TOKEN).await;
    let blank = app
        .connect_gitlab(owner, &workspace, "  ")
        .await
        .unwrap_err();
    let first = app.connect_gitlab(owner, &workspace, TOKEN).await;
    let second = app.connect_gitlab(owner, &workspace, TOKEN).await;

    // then
    assert_reason(invalid, "TOKEN_INVALID");
    assert_reason(narrow, "TOKEN_SCOPE_MISSING");
    assert_invalid_field(&blank, "token");
    assert!(first.is_ok());
    assert_reason(second, "CONNECTION_EXISTS");
}

/// A server with no key of the secrets cannot keep a GitLab token, and asks GitLab nothing.
#[sqlx::test(migrations = false)]
async fn connect_gitlab_needs_the_key_of_the_secrets(pool: PgPool) {
    // given
    let app = TestApp::spawn_without_secret_key(pool, scylla_ce::edition()).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();

    // when
    let refused = app.connect_gitlab(owner, &workspace, TOKEN).await;
    let providers = app.get_vcs_providers(owner, &workspace).await.unwrap();

    // then
    assert_reason(refused, "SECRET_KEY_MISSING");
    assert_eq!(providers[1].unavailable_reason, "SECRET_KEY_MISSING");
}

/// A new token of the same account replaces the sealed token and makes the connection active. A
/// token of another account and a GitHub connection are refused.
#[sqlx::test(migrations = false)]
async fn replace_gitlab_token_keeps_the_account(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace_id = app.workspace_of(owner).await;
    let workspace = workspace_id.to_string();
    let projects = [(1, "bank/payments", 40)];
    for user in [42, 42, 43] {
        fake.answer_gitlab_token((user, "scylla-bot", true), &["api"], Value::Null, &projects);
    }
    let made = app.connect_gitlab(owner, &workspace, TOKEN).await.unwrap();
    let id = made.connection.unwrap().id.unwrap().value;
    sqlx::query(
        "UPDATE vcs_connections SET state = 'needs_attention', state_reason = 'TOKEN_EXPIRED'",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let github = app
        .insert_github_connection(workspace_id, owner, 7, "acme")
        .await;

    // when
    let replaced = app
        .replace_gitlab_token(owner, &id, "glpat-second")
        .await
        .unwrap();
    let other = app.replace_gitlab_token(owner, &id, "glpat-third").await;
    let wrong = app
        .replace_gitlab_token(owner, &github, "glpat-third")
        .await
        .unwrap_err();

    // then
    let connection = replaced.connection.unwrap();
    assert_eq!(connection.id.unwrap().value, id);
    assert_eq!(connection.state, ConnectionState::Active as i32);
    assert_eq!(connection.state_reason, "");
    assert_eq!(opened(&app, &id).await, "glpat-second");
    assert_reason(other, "ACCOUNT_CHANGED");
    assert_invalid_field(&wrong, "connection_id");
}
