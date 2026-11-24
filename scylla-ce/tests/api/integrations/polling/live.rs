//! The tests of a poll of a real repository on GitHub.com and on GitLab.com. They are ignored:
//! they read the accounts of the `SCYLLA_LIVE_*` variables, and the dev GitHub App of the
//! `SCYLLA_GITHUB_APP_*` variables.

use scylla_core::api::ServeOptions;
use scylla_core::infra::vcs::{GitHubAppSettings, Hidden, VcsOptions};
use scylla_testkit::{text, LinkRow, TestApp};
use sqlx::PgPool;

/// Returns a variable of the environment of the test.
fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} for the live tests"))
}

/// Returns the dev GitHub App of the environment.
fn github_app() -> GitHubAppSettings {
    let key = match std::env::var("SCYLLA_GITHUB_APP_PRIVATE_KEY") {
        Ok(key) => key.replace("\\n", "\n"),
        Err(_) => std::fs::read_to_string(var("SCYLLA_GITHUB_APP_PRIVATE_KEY_FILE"))
            .expect("the private key of the App"),
    };
    GitHubAppSettings {
        app_id: var("SCYLLA_GITHUB_APP_ID").parse().expect("an App id"),
        slug: var("SCYLLA_GITHUB_APP_SLUG"),
        client_id: var("SCYLLA_GITHUB_APP_CLIENT_ID"),
        client_secret: Hidden::new(var("SCYLLA_GITHUB_APP_CLIENT_SECRET")),
        private_key: Hidden::new(key),
        webhook_secret: Hidden::new(var("SCYLLA_GITHUB_APP_WEBHOOK_SECRET")),
    }
}

/// Makes a project of a new owner, linked to a repository through a new connection.
async fn link(app: &TestApp, provider: &str, external_id: &str, full_name: &str) -> String {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "Live").await.unwrap();
    let project = text(&project.id).to_owned();
    let connection = match provider {
        "github" => {
            let installation = var("SCYLLA_LIVE_GITHUB_INSTALLATION_ID")
                .parse()
                .expect("an id");
            let account = full_name.split('/').next().unwrap_or_default();
            app.insert_github_connection(workspace, owner, installation, account)
                .await
        }
        _ => {
            let token = var("SCYLLA_LIVE_GITLAB_TOKEN");
            app.insert_gitlab_connection(workspace, owner, "scylla-bot", &token)
                .await
        }
    };
    let row = LinkRow {
        project: &project,
        connection: &connection,
        external_id,
        full_name,
        default_branch: "main",
    };
    app.insert_repository_link(&row).await;
    project
}

/// The first poll of the test repository of the dev GitHub App keeps the head of `main`, and
/// the next poll reads the branches again with their ETag.
#[ignore = "needs the dev GitHub App and SCYLLA_LIVE_GITHUB_*"]
#[sqlx::test(migrations = false)]
async fn a_poll_of_a_live_github_repository_keeps_its_heads(pool: PgPool) {
    // given
    let vcs = VcsOptions {
        github_app: Some(github_app()),
        ..VcsOptions::default()
    };
    let options = ServeOptions {
        vcs,
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    let repository = var("SCYLLA_LIVE_GITHUB_REPOSITORY");
    let id = var("SCYLLA_LIVE_GITHUB_REPOSITORY_ID");
    let project = link(&app, "github", &id, &repository).await;

    // when
    let since = app.poll_now(&project).await;
    app.wait_for_poll(&project, &since).await;
    let since = app.poll_now(&project).await;
    app.wait_for_poll(&project, &since).await;

    // then
    let heads = app.poll_heads(&project).await;
    assert!(
        heads.iter().any(|(name, _)| name == "refs/heads/main"),
        "{heads:?}"
    );
    assert!(app.deliveries().await.is_empty());
}

/// The first poll of the test project of the GitLab service account keeps the head of `main`.
#[ignore = "needs the GitLab service account and SCYLLA_LIVE_GITLAB_*"]
#[sqlx::test(migrations = false)]
async fn a_poll_of_a_live_gitlab_project_keeps_its_heads(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let id = var("SCYLLA_LIVE_GITLAB_PROJECT_ID");
    let project = link(&app, "gitlab", &id, &var("SCYLLA_LIVE_GITLAB_PROJECT")).await;

    // when
    let since = app.poll_now(&project).await;
    app.wait_for_poll(&project, &since).await;

    // then
    let heads = app.poll_heads(&project).await;
    assert!(
        heads.iter().any(|(name, _)| name == "refs/heads/main"),
        "{heads:?}"
    );
}
