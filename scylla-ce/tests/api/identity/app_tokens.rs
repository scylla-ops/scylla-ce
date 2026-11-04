//! The tests of the API tokens of the Apps: the secret one time, the exchange, the revocation,
//! the expiry, a disabled or deleted App, and who can change them.

use scylla_domain::identity::api_tokens::ApiTokenId;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::auth::v1::TokenExpiry;
use scylla_proto::projects::v1::ListProjectsRequest;
use scylla_testkit::{text, with_token, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Returns `Ok` if a token can call a route that each actor can call.
async fn try_token(app: &TestApp, token: &str) -> Result<(), Status> {
    let list = ListProjectsRequest {
        workspace_id: None,
        page: None,
    };
    app.projects
        .clone()
        .list_projects(with_token(token, list))
        .await
        .map(|_| ())
}

/// The secret comes one time with the prefix `scya_`, the list shows its hint only, and the
/// exchange gives a token of the App and records the use.
#[sqlx::test(migrations = false)]
async fn an_app_token_signs_in_as_its_app(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    let expiry = TokenExpiry::Days30 as i32;
    let created = app.create_app_token(owner, &key.app, "ci", expiry);
    let created = created.await.unwrap();

    // when
    let token = app.exchange_api_token(&created.secret).await.unwrap();

    // then
    assert!(created.secret.starts_with("scya_"));
    let made = created.token.unwrap();
    let end = &created.secret[created.secret.len() - 4..];
    assert_eq!(made.hint, format!("scya_...{end}"));
    assert_eq!(
        (made.name.as_str(), text(&made.actor_id)),
        ("ci", key.app.as_str())
    );
    let life = made.expires_at.unwrap().seconds - made.created_at.unwrap().seconds;
    assert_eq!(life, 30 * 24 * 60 * 60);
    assert!(try_token(&app, &token).await.is_ok());
    let listed = app.list_app_tokens(owner, &key.app).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].last_used_at.is_some() && listed[0].revoked_at.is_none());
}

/// A revoked token stops its short tokens at once and gives no new one. Only a token of the
/// App can be revoked.
#[sqlx::test(migrations = false)]
async fn a_revoked_app_token_stops_at_once(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    let (other, _) = app.app_in(owner, workspace, false).await;
    let created = app
        .create_app_token(owner, &key.app, "ci", 0)
        .await
        .unwrap();
    let id = text(&created.token.unwrap().id).to_owned();
    let token = app.exchange_api_token(&created.secret).await.unwrap();

    // when
    let elsewhere = app.revoke_app_token(owner, &other.app, &id).await;
    let unknown = app.revoke_app_token(owner, &key.app, UNKNOWN_ID).await;
    app.revoke_app_token(owner, &key.app, &id).await.unwrap();

    // then
    assert_code(elsewhere, Code::NotFound);
    assert_code(unknown, Code::NotFound);
    assert_code(try_token(&app, &token).await, Code::Unauthenticated);
    let again = app.exchange_api_token(&created.secret).await;
    assert_code(again, Code::Unauthenticated);
    let listed = app.list_app_tokens(owner, &key.app).await.unwrap();
    assert!(listed[0].revoked_at.is_some());
}

/// An expired token, a wrong prefix and an unknown secret give no token, and the short token
/// of an expired token stops.
#[sqlx::test(migrations = false)]
async fn an_expired_or_unknown_secret_is_refused(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    let created = app
        .create_app_token(owner, &key.app, "ci", 0)
        .await
        .unwrap();
    let token = app.exchange_api_token(&created.secret).await.unwrap();
    let id = ApiTokenId::parse(text(&created.token.unwrap().id)).unwrap();
    sqlx::query!(
        "UPDATE api_tokens SET created_at = now() - interval '2 days',
                               expires_at = now() - interval '1 day' WHERE id = $1",
        id.to_db(),
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let other_prefix = created.secret.replacen("scya_", "scyp_", 1);

    // when
    let expired = app.exchange_api_token(&created.secret).await;
    let prefix = app.exchange_api_token(&other_prefix).await;
    let unknown = app.exchange_api_token("scya_unknown").await;

    // then
    assert_code(expired, Code::Unauthenticated);
    assert_code(prefix, Code::Unauthenticated);
    assert_code(unknown, Code::Unauthenticated);
    assert_code(try_token(&app, &token).await, Code::Unauthenticated);
}

/// The token of a disabled App gets `APP_DISABLED`, and works again after the enable. A deleted
/// App loses its tokens.
#[sqlx::test(migrations = false)]
async fn a_disabled_or_deleted_app_gets_no_token(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    let created = app
        .create_app_token(owner, &key.app, "ci", 0)
        .await
        .unwrap();
    app.disable_app(owner, &key.app).await.unwrap();

    // when
    let disabled = app.exchange_api_token(&created.secret).await;
    app.enable_app(owner, &key.app).await.unwrap();
    let enabled = app.exchange_api_token(&created.secret).await.unwrap();
    let works = try_token(&app, &enabled).await;
    app.delete_app(owner, &key.app).await.unwrap();
    let deleted = app.exchange_api_token(&created.secret).await;

    // then
    let refused = disabled.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    let reason = refused.get_details_error_info().map(|info| info.reason);
    assert_eq!(reason.as_deref(), Some("APP_DISABLED"));
    assert!(works.is_ok());
    assert_code(deleted, Code::Unauthenticated);
}

/// The admins create and revoke the tokens, the readers list them, and a bad name or expiry
/// is refused.
#[sqlx::test(migrations = false)]
async fn the_app_token_routes_are_checked(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    let created = app
        .create_app_token(owner, &key.app, "ci", 0)
        .await
        .unwrap();
    let id = text(&created.token.unwrap().id).to_owned();

    // when
    let create = app.create_app_token(member, &key.app, "ci", 0).await;
    let list = app.list_app_tokens(member, &key.app).await;
    let revoke = app.revoke_app_token(member, &key.app, &id).await;
    let no_name = app.create_app_token(owner, &key.app, " ", 0).await;
    let bad_expiry = app.create_app_token(owner, &key.app, "ci", 99).await;

    // then
    assert_code(create, Code::PermissionDenied);
    assert_eq!(list.unwrap().len(), 1);
    assert_code(revoke, Code::PermissionDenied);
    assert_invalid_field(&no_name.unwrap_err(), "name");
    assert_invalid_field(&bad_expiry.unwrap_err(), "expiry");
}
