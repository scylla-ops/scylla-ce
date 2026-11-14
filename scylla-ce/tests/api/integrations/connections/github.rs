//! The tests of the install of the GitHub App in a workspace, on the fake provider.

use scylla_proto::integrations::v1::{
    AccountKind, AuthKind, CompleteGitHubInstallRequest, ConnectionState, VcsProvider,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use super::{app_with_fake, assert_reason, permissions, state_of};
use crate::{app, assert_invalid_field};

/// Makes the redirect of GitHub after an install.
fn redirect(state: &str, installation: &str, action: &str) -> CompleteGitHubInstallRequest {
    CompleteGitHubInstallRequest {
        state: state.to_owned(),
        installation_id: installation.to_owned(),
        code: "good-code".to_owned(),
        setup_action: action.to_owned(),
    }
}

/// The install page is the page of the App on GitHub with a random state. The database keeps
/// the hash of the state only.
#[sqlx::test(migrations = false)]
async fn start_github_install_gives_the_install_page_with_a_state(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();

    // when
    let started = app.start_github_install(owner, &workspace).await.unwrap();

    // then
    let page = format!("{}/apps/scylla-test/installations/new?state=", fake.url());
    assert!(started.install_url.starts_with(&page), "{started:?}");
    let state = state_of(&started.install_url);
    assert!(state.len() >= 43);
    let hash = Sha256::digest(state.as_bytes()).to_vec();
    let kept: Vec<Vec<u8>> = sqlx::query_scalar("SELECT hash FROM vcs_setup_states")
        .fetch_all(&app.pool)
        .await
        .unwrap();
    assert_eq!(kept, [hash]);
    let expires = started.expires_at.expect("an expiry").seconds;
    let now: i64 = sqlx::query_scalar("SELECT extract(epoch FROM now())::bigint")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(
        (now + 590..=now + 610).contains(&expires),
        "{expires} {now}"
    );
}

/// A server with no GitHub App cannot start an install.
#[sqlx::test(migrations = false)]
async fn start_github_install_needs_an_app(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();

    // when
    let refused = app.start_github_install(owner, &workspace).await;

    // then
    assert_reason(refused, "APP_MISSING");
}

/// The redirect makes the connection of the installation: the code of the user, the check that
/// the user sees the installation, then the installation read with the JWT of the App. A state
/// serves one time.
#[sqlx::test(migrations = false)]
async fn complete_github_install_makes_the_connection(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let repositories = [(1, "acme/api", false), (2, "acme/web", true)];
    fake.answer_github_install(7, "acme", permissions(), &repositories);
    let started = app.start_github_install(owner, &workspace).await.unwrap();
    let state = state_of(&started.install_url);

    // when
    let made = app
        .complete_github_install(owner, redirect(&state, "7", "install"))
        .await
        .unwrap();
    let again = app
        .complete_github_install(owner, redirect(&state, "7", "install"))
        .await;

    // then
    assert_eq!(made.provider, VcsProvider::Github as i32);
    assert_eq!(made.auth_kind, AuthKind::AppInstallation as i32);
    assert_eq!(made.account_kind, AccountKind::Organization as i32);
    assert_eq!(made.state, ConnectionState::Active as i32);
    assert_eq!(
        (made.account.as_str(), made.external_id.as_str()),
        ("acme", "7")
    );
    assert_eq!((made.repository_count, made.all_repositories), (2, false));
    assert_eq!(made.missing_permissions, ["administration"]);
    assert_eq!(made.avatar_url, "https://avatars.example/acme");
    let settings = format!("{}/organizations/acme/settings/installations/7", fake.url());
    assert_eq!(made.settings_url, settings);
    assert_eq!(made.workspace_id.as_ref().unwrap().value, workspace);
    assert_reason(again, "STATE_EXPIRED");
    let code = &fake.requests_to("POST", "/login/oauth/access_token")[0];
    assert_eq!(code.body["code"], "good-code");
    let listed = &fake.requests_to("GET", "/user/installations")[0];
    assert_eq!(listed.header("authorization"), Some("Bearer ghu_user"));
    let read = &fake.requests_to("GET", "/app/installations/7")[0];
    let jwt = read.header("authorization").unwrap_or_default();
    assert!(jwt.starts_with("Bearer ey"), "{jwt}");
}

/// A state of another user, an unknown state, or a code that GitHub refuses give
/// `STATE_EXPIRED`. A bad installation id is a bad request.
#[sqlx::test(migrations = false)]
async fn complete_github_install_refuses_a_foreign_state_or_a_bad_code(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let refused = json!({ "error": "bad_verification_code" });
    fake.respond("POST", "/login/oauth/access_token", 200, refused, &[]);
    let first = app.start_github_install(owner, &workspace).await.unwrap();
    let first = state_of(&first.install_url);

    // when
    let foreign = app
        .complete_github_install(stranger, redirect(&first, "7", "install"))
        .await;
    let unknown = app
        .complete_github_install(owner, redirect("unknown", "7", "install"))
        .await;
    let bad_id = app
        .complete_github_install(owner, redirect(&first, "seven", "install"))
        .await
        .unwrap_err();
    let bad_code = app
        .complete_github_install(owner, redirect(&first, "7", "install"))
        .await;

    // then
    assert_reason(foreign, "STATE_EXPIRED");
    assert_reason(unknown, "STATE_EXPIRED");
    assert_invalid_field(&bad_id, "installation_id");
    assert_reason(bad_code, "STATE_EXPIRED");
}

/// A request that waits for an owner, a cancelled install, and an installation that the user
/// does not see make no connection.
#[sqlx::test(migrations = false)]
async fn complete_github_install_tells_why_it_makes_no_connection(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    fake.answer_github_install(7, "acme", permissions(), &[]);
    let mut states = Vec::new();
    for _ in 0..3 {
        let started = app.start_github_install(owner, &workspace).await.unwrap();
        states.push(state_of(&started.install_url));
    }

    // when
    let pending = app
        .complete_github_install(owner, redirect(&states[0], "", "request"))
        .await;
    let cancelled = app
        .complete_github_install(owner, redirect(&states[1], "", "install"))
        .await;
    let hidden = app
        .complete_github_install(owner, redirect(&states[2], "8", "install"))
        .await;

    // then
    assert_reason(pending, "INSTALL_PENDING_APPROVAL");
    assert_reason(cancelled, "INSTALL_CANCELLED");
    assert_reason(hidden, "INSTALLATION_NOT_VISIBLE");
    let page = app.list_vcs_connections(owner, &workspace, 0, "").await;
    assert!(page.unwrap().connections.is_empty());
}

/// A second install of the same installation refreshes its connection: a suspended App gives a
/// suspended connection with no repository count.
#[sqlx::test(migrations = false)]
async fn complete_github_install_refreshes_the_connection(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    fake.answer_github_install(7, "acme", permissions(), &[(1, "acme/api", false)]);
    let suspended = json!({
        "id": 7,
        "account": { "login": "acme-renamed", "avatar_url": null, "type": "Organization" },
        "repository_selection": "all",
        "permissions": permissions(),
        "suspended_at": "2026-11-01T10:00:00Z",
    });
    fake.respond("GET", "/app/installations/7", 200, suspended, &[]);
    let mut made = Vec::new();

    // when
    for _ in 0..2 {
        let started = app.start_github_install(owner, &workspace).await.unwrap();
        let state = state_of(&started.install_url);
        let redirect = redirect(&state, "7", "update");
        made.push(app.complete_github_install(owner, redirect).await.unwrap());
    }

    // then
    assert_eq!(made[0].id, made[1].id);
    assert_eq!(made[1].account, "acme-renamed");
    assert_eq!(made[1].state, ConnectionState::Suspended as i32);
    assert_eq!(made[1].state_reason, "APP_SUSPENDED");
    assert_eq!(
        (made[1].repository_count, made[1].all_repositories),
        (0, true)
    );
}
