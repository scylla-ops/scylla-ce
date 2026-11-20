//! The tests of the choice of an installation after the authorize page of a new install, on the
//! fake provider: the installations that the user sees, and the connection of one of them.

use scylla_proto::integrations::v1::{AccountKind, CompleteGitHubInstallRequest, ConnectionState};
use serde_json::json;
use sqlx::PgPool;

use super::{app_with_fake, assert_reason, permissions, state_of};

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

/// Story: the App is installed on an organization, and the installation has a connection in the
/// workspace of a first admin. A second admin that sees the installation on GitHub starts an
/// install in another workspace. After the authorize page, the server gives this installation
/// to choose. Its page carries the installation in the state, GitHub comes back from there with
/// a code, and the installation then has a connection in each workspace. The state is used.
#[sqlx::test(migrations = false)]
async fn complete_github_install_connects_an_installation_of_another_workspace(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let first = app.user().await;
    let second = app.user().await;
    let first_workspace = app.workspace_of(first).await.to_string();
    let second_workspace = app.workspace_of(second).await.to_string();
    fake.answer_github_install(7, "acme", permissions(), &[(1, "acme/api", false)]);
    let kept = app
        .install_github(first, &first_workspace, 7)
        .await
        .unwrap();
    let started = app
        .start_github_install(second, &second_workspace)
        .await
        .unwrap();
    let state = state_of(&started.install_url);

    // when
    let choice = app
        .choose_github_installation(second, redirect(&state, "", "good-code", ""))
        .await
        .unwrap();
    let option = &choice.installations[0];
    let back = redirect(&state_of(&option.connect_url), "", "good-code", "");
    let made = app.complete_github_install(second, back).await.unwrap();
    let used = app
        .choose_github_installation(second, redirect(&state, "", "good-code", ""))
        .await;

    // then
    assert_eq!(choice.installations.len(), 1);
    assert_eq!(
        (option.installation_id.as_str(), option.account.as_str()),
        ("7", "acme")
    );
    assert_eq!(option.account_kind(), AccountKind::Organization);
    assert_eq!(option.avatar_url, "https://avatars.example/acme");
    let page = format!("{}/login/oauth/authorize?client_id=Iv1.test&", fake.url());
    assert!(option.connect_url.starts_with(&page), "{option:?}");
    assert_eq!(state_of(&option.connect_url), format!("{state}.7"));
    assert_eq!(made.external_id, "7");
    assert_eq!(made.workspace_id.as_ref().unwrap().value, second_workspace);
    assert_eq!(made.state(), ConnectionState::Active);
    assert_ne!(made.id, kept.id);
    let page = app
        .list_vcs_connections(first, &first_workspace, 0, "")
        .await;
    let page = page.unwrap();
    assert_eq!(page.connections.len(), 1);
    assert_eq!(page.connections[0].id, kept.id);
    assert_eq!(page.connections[0].state(), ConnectionState::Active);
    assert_reason(used, "STATE_EXPIRED");
}

/// After the authorize page, the server lists the installations that the user sees on GitHub
/// and that have no connection in the workspace, in the order of GitHub, with the install page
/// and the same state. The list does not use the state: the user can go back to it.
#[sqlx::test(migrations = false)]
async fn complete_github_install_lists_the_installations_with_no_connection(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    fake.answer_github_install(7, "acme", permissions(), &[]);
    let seen = [
        (9, "octocat", "User"),
        (7, "acme", "Organization"),
        (8, "globex", "Organization"),
    ];
    fake.answer_github_user(&seen);
    app.install_github(owner, &workspace, 7).await.unwrap();
    let started = app.start_github_install(owner, &workspace).await.unwrap();
    let state = state_of(&started.install_url);

    // when
    let choice = app
        .choose_github_installation(owner, redirect(&state, "", "good-code", ""))
        .await
        .unwrap();
    let again = app
        .choose_github_installation(owner, redirect(&state, "", "good-code", ""))
        .await
        .unwrap();

    // then
    let listed: Vec<(&str, &str, AccountKind)> = choice
        .installations
        .iter()
        .map(|option| {
            let id = option.installation_id.as_str();
            (id, option.account.as_str(), option.account_kind())
        })
        .collect();
    let wanted = [
        ("9", "octocat", AccountKind::User),
        ("8", "globex", AccountKind::Organization),
    ];
    assert_eq!(listed, wanted);
    let page = format!("{}/apps/scylla-test/installations/new?state=", fake.url());
    assert!(choice.install_url.starts_with(&page), "{choice:?}");
    assert_eq!(state_of(&choice.install_url), state);
    assert_eq!(again, choice);
}

/// A user with no installation to connect gets the install page only. A code that GitHub
/// refuses and the state of another user give `STATE_EXPIRED`, with no call to GitHub for the
/// state of another user.
#[sqlx::test(migrations = false)]
async fn complete_github_install_gives_the_install_page_when_no_installation_is_left(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    fake.answer_github_user(&[]);
    let refused = json!({ "error": "bad_verification_code" });
    fake.respond("POST", "/login/oauth/access_token", 200, refused, &[]);
    let started = app.start_github_install(owner, &workspace).await.unwrap();
    let state = state_of(&started.install_url);

    // when
    let foreign = app
        .choose_github_installation(stranger, redirect(&state, "", "good-code", ""))
        .await;
    let choice = app
        .choose_github_installation(owner, redirect(&state, "", "good-code", ""))
        .await
        .unwrap();
    let bad_code = app
        .choose_github_installation(owner, redirect(&state, "", "bad-code", ""))
        .await;

    // then
    assert_reason(foreign, "STATE_EXPIRED");
    assert!(choice.installations.is_empty());
    assert_eq!(state_of(&choice.install_url), state);
    assert_reason(bad_code, "STATE_EXPIRED");
    assert_eq!(
        fake.requests_to("POST", "/login/oauth/access_token").len(),
        2
    );
}
