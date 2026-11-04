//! The tests of the personal tokens: they act as their user inside their limits, they cannot
//! make another token, and the switches of the instance and of a workspace stop them.

use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_proto::auth::v1::{TokenLimits, TokenPreset};
use scylla_proto::projects::v1::ListProjectsRequest;
use scylla_proto::workspaces::v1::get_workspace_request::Workspace as Named;
use scylla_proto::workspaces::v1::workspace_service_client::WorkspaceServiceClient;
use scylla_proto::workspaces::v1::GetWorkspaceRequest;
use scylla_testkit::{id, text, with_token, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::{app, assert_code, UNKNOWN_ID};

/// Returns `Ok` if a token can call a route that each actor can call.
async fn try_token(app: &TestApp, token: &str) -> Result<(), Status> {
    let list = ListProjectsRequest {
        workspace_id: None,
        page: None,
    };
    let mut client = app.projects.clone();
    client.list_projects(with_token(token, list)).await?;
    Ok(())
}

/// Calls `GetWorkspace` with a token.
async fn get_workspace(app: &TestApp, token: &str, workspace: WorkspaceId) -> Result<(), Status> {
    let request = GetWorkspaceRequest {
        workspace: Some(Named::WorkspaceId(id(workspace.to_string()))),
    };
    let mut client = WorkspaceServiceClient::new(app.channel.clone());
    client.get_workspace(with_token(token, request)).await?;
    Ok(())
}

/// Makes the limits of the full preset.
fn full() -> Option<TokenLimits> {
    Some(TokenLimits {
        preset: TokenPreset::Full as i32,
        ..TokenLimits::default()
    })
}

/// Returns the reason of a refusal.
fn reason(status: &Status) -> Option<String> {
    status.get_details_error_info().map(|info| info.reason)
}

/// A personal token starts with `scyp_`, acts as its user, and stops at its revocation. Only
/// its user revokes it.
#[sqlx::test(migrations = false)]
async fn a_personal_token_acts_as_its_user(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (user, other) = (app.user().await, app.user().await);
    let created = app
        .create_personal_token(user, "laptop", None)
        .await
        .unwrap();
    let id = text(&created.token.as_ref().unwrap().id).to_owned();

    // when
    let token = app.exchange_api_token(&created.secret).await.unwrap();
    let works = try_token(&app, &token).await;
    let listed = app.list_personal_tokens(user).await.unwrap();
    let by_other = app.revoke_personal_token(other, &id).await;
    let unknown = app.revoke_personal_token(user, UNKNOWN_ID).await;
    app.revoke_personal_token(user, &id).await.unwrap();

    // then
    assert!(created.secret.starts_with("scyp_"));
    let made = created.token.unwrap();
    assert_eq!(text(&made.actor_id), user.to_string());
    assert!(works.is_ok());
    assert!(listed.enabled && listed.refused_workspace_ids.is_empty());
    assert_eq!(listed.tokens.len(), 1);
    assert_code(by_other, Code::NotFound);
    assert_code(unknown, Code::NotFound);
    assert_code(try_token(&app, &token).await, Code::Unauthenticated);
}

/// A token that came from an API token cannot create a personal token, even with the full
/// preset.
#[sqlx::test(migrations = false)]
async fn an_api_token_cannot_make_another_one(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let created = app.create_personal_token(user, "ci", full()).await.unwrap();
    let token = app.exchange_api_token(&created.secret).await.unwrap();

    // when
    let made = app.create_personal_token_with(&token, "more", full()).await;

    // then
    assert_code(made, Code::PermissionDenied);
}

/// When an instance admin turns the personal tokens off, their short tokens stop at once, and
/// no exchange or creation passes until they are on again.
#[sqlx::test(migrations = false)]
async fn the_instance_switch_stops_the_personal_tokens(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (admin, user) = (app.admin().await, app.user().await);
    let created = app.create_personal_token(user, "ci", None).await.unwrap();
    let token = app.exchange_api_token(&created.secret).await.unwrap();

    // when
    let by_user = app.set_personal_tokens(user, false).await;
    let off = app.set_personal_tokens(admin, false).await.unwrap();
    let stopped = try_token(&app, &token).await;
    let exchange = app.exchange_api_token(&created.secret).await;
    let create = app.create_personal_token(user, "more", None).await;
    let listed = app.list_personal_tokens(user).await.unwrap();
    app.set_personal_tokens(admin, true).await.unwrap();
    let again = app.exchange_api_token(&created.secret).await;

    // then
    assert_code(by_user, Code::PermissionDenied);
    assert!(!off.personal_tokens_enabled);
    assert_eq!(off.active_personal_tokens, 1);
    assert_code(stopped, Code::Unauthenticated);
    let exchange = exchange.unwrap_err();
    assert_eq!(exchange.code(), Code::FailedPrecondition);
    assert_eq!(
        reason(&exchange).as_deref(),
        Some("PERSONAL_TOKENS_DISABLED")
    );
    let create = create.unwrap_err();
    assert_eq!(reason(&create).as_deref(), Some("PERSONAL_TOKENS_DISABLED"));
    assert!(!listed.enabled);
    assert!(try_token(&app, &again.unwrap()).await.is_ok());
    let settings = app.get_instance_settings(admin).await.unwrap();
    assert!(settings.personal_tokens_enabled);
}

/// A workspace that refuses the personal tokens refuses each of their calls on it, and only on
/// it. The other tokens of the user still work there, and only an admin changes the switch.
#[sqlx::test(migrations = false)]
async fn a_workspace_can_refuse_the_personal_tokens(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member) = (app.user().await, app.user().await);
    let refusing = app.workspace_of(owner).await;
    let second = app.create_workspace(owner, "second").await.unwrap();
    let second = WorkspaceId::parse(text(&second.id)).unwrap();
    app.grant(&[Workspace::MEMBER.of(refusing, member)]).await;
    let created = app
        .create_personal_token(owner, "ci", full())
        .await
        .unwrap();
    let token = app.exchange_api_token(&created.secret).await.unwrap();
    let session = app.token(owner).await;

    // when
    let by_member = app
        .set_workspace_personal_tokens(member, refusing, false)
        .await;
    let off = app
        .set_workspace_personal_tokens(owner, refusing, false)
        .await;
    let refused = get_workspace(&app, &token, refusing).await;
    let other = get_workspace(&app, &token, second).await;
    let by_session = get_workspace(&app, &session, refusing).await;
    let listed = app.list_personal_tokens(owner).await.unwrap();

    // then
    assert_code(by_member, Code::PermissionDenied);
    assert!(!off.unwrap());
    assert_code(refused, Code::PermissionDenied);
    assert!(other.is_ok() && by_session.is_ok());
    let ids: Vec<_> = listed
        .refused_workspace_ids
        .iter()
        .map(|id| id.value.clone())
        .collect();
    assert_eq!(ids, vec![refusing.to_string()]);
}
