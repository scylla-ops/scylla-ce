//! The tests of the export of the data of an account: the rows of the caller, with no secret and
//! no data of another user.

use scylla_testkit::{Role, TestApp};
use serde_json::Value;
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::PASSWORD;
use crate::{app, assert_code};

/// Signs in a user with a password, and returns its access token.
async fn token_of(app: &TestApp, email: &str) -> String {
    app.sign_in(email, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value
}

/// Calls `ExportAccountData` and reads its JSON document.
async fn export(app: &TestApp, token: &str) -> (String, Value) {
    let export = app.export_account_data(token).await.unwrap();
    assert!(export.exported_at.is_some());
    let data = serde_json::from_str(&export.json).unwrap();
    (export.json, data)
}

/// The export holds the account, the emails, the sessions, the personal tokens, the state of the
/// web app, the owned workspaces and the roles of the caller, and no secret.
#[sqlx::test(migrations = false)]
async fn export_account_data_gives_the_rows_of_the_caller(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "ada@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    let token = token_of(&app, email).await;
    let personal = app.workspace_of(user).await;
    let created = app.create_personal_token(user, "laptop", None).await;
    app.put_user_state(user, "theme", "\"dark\"", None)
        .await
        .unwrap();
    let owner = app.user().await;
    let shared = app.workspace_of(owner).await.to_string();
    let viewer = Role::on("workspace", &shared, "viewer");
    app.grant_role(owner, viewer, &user.to_string())
        .await
        .unwrap();

    // when
    let (json, data) = export(&app, &token).await;

    // then
    assert_eq!(data["format"], "scylla.account-data.v1");
    assert_eq!(data["account"]["id"], user.to_string());
    assert_eq!(data["account"]["status"], "active");
    assert!(data["account"]["password_changed_at"].is_string());
    assert_eq!(data["emails"][0]["email"], email);
    assert_eq!(data["emails"][0]["primary"], true);
    assert!(data["sessions"][0]["created_at"].is_string());
    assert_eq!(data["personal_tokens"][0]["name"], "laptop");
    assert_eq!(data["interface_state"][0]["key"], "theme");
    assert_eq!(data["interface_state"][0]["value"], "dark");
    assert_eq!(data["owned_workspaces"][0]["id"], personal.to_string());
    assert_eq!(data["roles"][0]["scope"], "workspace");
    assert_eq!(data["roles"][0]["scope_id"], shared);
    assert_eq!(data["roles"][0]["role"], "viewer");
    assert!(data["edition"].as_object().unwrap().is_empty());
    assert!(!json.contains(&created.unwrap().secret));
    assert!(!json.contains("argon2"));
}

/// The export holds no data of another user.
#[sqlx::test(migrations = false)]
async fn export_account_data_has_no_data_of_another_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    app.user_with("ada@example.com", Some(PASSWORD)).await;
    let other = app.user_with("grace@example.com", Some(PASSWORD)).await;
    token_of(&app, "grace@example.com").await;
    app.put_user_state(other, "theme", "\"light\"", None)
        .await
        .unwrap();
    let token = token_of(&app, "ada@example.com").await;

    // when
    let (json, data) = export(&app, &token).await;

    // then
    assert!(!json.contains("grace@example.com"));
    assert!(!json.contains(&other.to_string()));
    assert_eq!(data["sessions"].as_array().unwrap().len(), 1);
    assert!(data["interface_state"].as_array().unwrap().is_empty());
}

/// A deleted account gives no export: its tokens stop.
#[sqlx::test(migrations = false)]
async fn export_account_data_needs_an_account(pool: PgPool) {
    // given
    let app = app(pool).await;
    app.user_with("ada@example.com", Some(PASSWORD)).await;
    let token = token_of(&app, "ada@example.com").await;
    app.delete_account(&token, PASSWORD).await.unwrap();

    // when
    let result = app.export_account_data(&token).await;

    // then
    assert_code(result, Code::Unauthenticated);
}
