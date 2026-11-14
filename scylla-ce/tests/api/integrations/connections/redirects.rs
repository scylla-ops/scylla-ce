//! The tests of the redirects of GitHub that are not a new install, on the fake provider: an
//! install on an account where the App is installed (no code), and a change of an installation
//! on GitHub (no state).

use std::collections::HashMap;

use reqwest::Url;
use scylla_proto::integrations::v1::{CompleteGitHubInstallRequest, ConnectionState};
use scylla_testkit::text;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use super::{app_with_fake, assert_reason, permissions, state_of};
use crate::{assert_code, assert_invalid_field};

/// Makes a redirect of GitHub.
fn redirect(
    state: &str,
    installation: &str,
    code: &str,
    action: &str,
) -> CompleteGitHubInstallRequest {
    CompleteGitHubInstallRequest {
        state: state.to_owned(),
        installation_id: installation.to_owned(),
        code: code.to_owned(),
        setup_action: action.to_owned(),
    }
}

/// Returns the query of the authorize page of a refusal `AUTHORIZATION_REQUIRED`, and the page
/// with no query.
fn authorize_page(status: &Status) -> (String, HashMap<String, String>) {
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo");
    assert_eq!(info.reason, "AUTHORIZATION_REQUIRED", "{status:?}");
    let url = Url::parse(&info.metadata["authorize_url"]).expect("a URL");
    let query = url.query_pairs().into_owned().collect();
    let page = format!("{}{}", url.origin().ascii_serialization(), url.path());
    (page, query)
}

/// Story: an admin connects an account where the App is installed. GitHub comes back with the
/// installation and no code, so the server sends the user to the authorize page of the App with
/// the state and the installation. GitHub comes back from there with a code, and the install
/// completes with the check that the user sees the installation.
#[sqlx::test(migrations = false)]
async fn complete_github_install_with_no_code_goes_to_the_authorize_page(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    fake.answer_github_install(7, "acme", permissions(), &[(1, "acme/api", false)]);
    let started = app.start_github_install(owner, &workspace).await.unwrap();
    let state = state_of(&started.install_url);

    // when
    let refused = app
        .complete_github_install(owner, redirect(&state, "7", "", "update"))
        .await
        .unwrap_err();
    let (page, query) = authorize_page(&refused);
    let back = redirect(&query["state"], "", "good-code", "");
    let made = app.complete_github_install(owner, back.clone()).await;
    let again = app.complete_github_install(owner, back).await;

    // then
    assert_eq!(page, format!("{}/login/oauth/authorize", fake.url()));
    assert_eq!(query["client_id"], "Iv1.test");
    assert!(query["redirect_uri"].ends_with("/integrations/github/callback"));
    assert_eq!(query["state"], format!("{state}.7"));
    let made = made.unwrap();
    assert_eq!(
        (made.account.as_str(), made.external_id.as_str()),
        ("acme", "7")
    );
    assert_eq!(made.workspace_id.as_ref().unwrap().value, workspace);
    assert_eq!(made.state(), ConnectionState::Active);
    assert_reason(again, "STATE_EXPIRED");
    assert_eq!(
        fake.requests_to("POST", "/login/oauth/access_token").len(),
        1
    );
    assert_eq!(fake.requests_to("GET", "/user/installations").len(), 1);
}

/// A user that refuses the authorization comes back with the state and no code: the install is
/// cancelled and the state is used. A state with a bad installation is an expired one, and a
/// request with no state, no code or no installation is a bad request.
#[sqlx::test(migrations = false)]
async fn complete_github_install_tells_a_refused_authorization(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    fake.answer_github_install(7, "acme", permissions(), &[]);
    let started = app.start_github_install(owner, &workspace).await.unwrap();
    let state = state_of(&started.install_url);
    let carried = format!("{state}.7");

    // when
    let denied = app
        .complete_github_install(owner, redirect(&carried, "", "", ""))
        .await;
    let again = app
        .complete_github_install(owner, redirect(&carried, "", "good-code", ""))
        .await;
    let bad = app
        .complete_github_install(owner, redirect(&format!("{state}.x"), "", "good-code", ""))
        .await;
    let empty = app
        .complete_github_install(owner, redirect("", "7", "", "update"))
        .await
        .unwrap_err();

    // then
    assert_reason(denied, "INSTALL_CANCELLED");
    assert_reason(again, "STATE_EXPIRED");
    assert_reason(bad, "STATE_EXPIRED");
    assert_invalid_field(&empty, "state");
    assert!(fake
        .requests_to("POST", "/login/oauth/access_token")
        .is_empty());
}

/// Story: an owner changes the repositories of the installation on GitHub. GitHub comes back
/// with a code, the installation and no state, and the server refreshes the connection of the
/// installation in the workspace of the caller. A user with no connection of the installation
/// that they manage gets the answer of a refusal, and one that does not see the installation on
/// GitHub gets `INSTALLATION_NOT_VISIBLE`.
#[sqlx::test(migrations = false)]
async fn complete_github_install_with_no_state_refreshes_the_connection(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let workspace_id = app.workspace_of(owner).await;
    let workspace = workspace_id.to_string();
    fake.answer_github_install(7, "acme", permissions(), &[(1, "acme/api", false)]);
    let repositories = [(1, "acme/api", false), (2, "acme/web", false)];
    fake.answer_github_install(7, "acme", permissions(), &repositories);
    let first = app.install_github(owner, &workspace, 7).await.unwrap();
    app.insert_github_connection(workspace_id, owner, 9, "labs")
        .await;
    let change = redirect("", "7", "good-code", "update");

    // when
    let refreshed = app.complete_github_install(owner, change.clone()).await;
    let refused = app.complete_github_install(stranger, change).await;
    let unknown = app
        .complete_github_install(owner, redirect("", "8", "good-code", "update"))
        .await;
    let hidden = app
        .complete_github_install(owner, redirect("", "9", "good-code", "update"))
        .await;
    let listed = app.list_vcs_connections(owner, &workspace, 0, "").await;

    // then
    let refreshed = refreshed.unwrap();
    assert_eq!(text(&refreshed.id), text(&first.id));
    assert_eq!(first.repository_count, 1);
    assert_eq!(refreshed.repository_count, 2);
    assert_eq!(refreshed.workspace_id.as_ref().unwrap().value, workspace);
    assert_code(refused, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_reason(hidden, "INSTALLATION_NOT_VISIBLE");
    assert_eq!(listed.unwrap().connections.len(), 2);
}
