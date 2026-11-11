//! The tests of the pipeline files of a real repository on GitHub.com and GitLab.com. They are
//! ignored: they read the accounts of the `SCYLLA_LIVE_*` variables, and the dev GitHub App of the
//! `SCYLLA_GITHUB_APP_*` variables.

use scylla_core::api::ServeOptions;
use scylla_core::infra::vcs::{GitHubAppSettings, Hidden, VcsOptions};
use scylla_testkit::{LinkRow, TestApp};
use sqlx::PgPool;

use super::linked;

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

/// Returns `true` for a full commit SHA.
fn is_sha(text: &str) -> bool {
    text.len() == 40 && text.chars().all(|char| char.is_ascii_hexdigit())
}

/// The pipeline files of the default branch of the test repository of the dev GitHub App read
/// with no error of the provider.
#[ignore = "needs the dev GitHub App and SCYLLA_LIVE_GITHUB_*"]
#[sqlx::test(migrations = false)]
async fn list_pipeline_files_of_a_live_github_repository(pool: PgPool) {
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
    let project = linked(&app, "none").await;
    let installation = var("SCYLLA_LIVE_GITHUB_INSTALLATION")
        .parse()
        .expect("an id");
    let repository = var("SCYLLA_LIVE_GITHUB_REPOSITORY");
    let account = repository.split('/').next().unwrap_or_default();
    let connection = app
        .insert_github_connection(project.workspace, project.owner, installation, account)
        .await;
    let link = LinkRow {
        project: &project.project,
        connection: &connection,
        external_id: &var("SCYLLA_LIVE_GITHUB_REPOSITORY_ID"),
        full_name: &repository,
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;

    // when
    let listed = app
        .list_pipeline_files(project.owner, &project.project, None)
        .await;

    // then
    let listed = listed.unwrap();
    assert!(is_sha(&listed.commit_sha), "{}", listed.commit_sha);
    assert!(listed.files.iter().all(|file| file.content.is_empty()));
}

/// The pipeline files of the default branch of the test project of the GitLab service account
/// read with no error of the provider.
#[ignore = "needs the GitLab service account and SCYLLA_LIVE_GITLAB_*"]
#[sqlx::test(migrations = false)]
async fn list_pipeline_files_of_a_live_gitlab_project(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let project = linked(&app, "none").await;
    let token = var("SCYLLA_LIVE_GITLAB_TOKEN");
    let connection = app
        .insert_gitlab_connection(project.workspace, project.owner, "scylla-bot", &token)
        .await;
    let link = LinkRow {
        project: &project.project,
        connection: &connection,
        external_id: &var("SCYLLA_LIVE_GITLAB_PROJECT_ID"),
        full_name: &var("SCYLLA_LIVE_GITLAB_PROJECT"),
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;

    // when
    let listed = app
        .list_pipeline_files(project.owner, &project.project, None)
        .await;

    // then
    let listed = listed.unwrap();
    assert!(is_sha(&listed.commit_sha), "{}", listed.commit_sha);
    assert!(listed.files.iter().all(|file| file.content.is_empty()));
}
