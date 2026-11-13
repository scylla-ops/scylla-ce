//! The tests of a commit status on a real repository on GitHub.com and GitLab.com. They are
//! ignored: they read the accounts of the `SCYLLA_LIVE_*` variables, and the dev GitHub App of
//! the `SCYLLA_GITHUB_APP_*` variables.

use scylla_core::api::ServeOptions;
use scylla_core::infra::vcs::{GitHubAppSettings, Hidden, VcsOptions};
use scylla_proto::integrations::v1::ReportState;
use scylla_testkit::{LinkRow, TestApp};
use sqlx::PgPool;

use super::{linked, run_on_main, Linked};

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

/// Links a project of a test to a live repository through a connection.
async fn link(app: &TestApp, project: &mut Linked, connection: String, id: &str, name: &str) {
    let link = LinkRow {
        project: &project.project,
        connection: &connection,
        external_id: id,
        full_name: name,
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;
    project.connection = connection;
}

/// A run on the default branch of the test repository of the dev GitHub App posts its commit
/// status.
#[ignore = "needs the dev GitHub App and SCYLLA_LIVE_GITHUB_*"]
#[sqlx::test(migrations = false)]
async fn a_live_github_commit_gets_the_status_of_a_run(pool: PgPool) {
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
    let mut project = linked(&app, "none").await;
    let installation = var("SCYLLA_LIVE_GITHUB_INSTALLATION")
        .parse()
        .expect("an id");
    let repository = var("SCYLLA_LIVE_GITHUB_REPOSITORY");
    let account = repository.split('/').next().unwrap_or_default();
    let connection = app
        .insert_github_connection(project.workspace, project.owner, installation, account)
        .await;
    let id = var("SCYLLA_LIVE_GITHUB_REPOSITORY_ID");
    link(&app, &mut project, connection, &id, &repository).await;

    // when
    let job = run_on_main(&app, &project).await;
    let report = app
        .wait_for_report(project.owner, &job, ReportState::Sent)
        .await;

    // then
    assert_eq!(report.sent_state, "pending");
}

/// A run on the default branch of the test project of the GitLab service account posts its
/// commit status.
#[ignore = "needs the GitLab service account and SCYLLA_LIVE_GITLAB_*"]
#[sqlx::test(migrations = false)]
async fn a_live_gitlab_commit_gets_the_status_of_a_run(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let mut project = linked(&app, "none").await;
    let token = var("SCYLLA_LIVE_GITLAB_TOKEN");
    let connection = app
        .insert_gitlab_connection(project.workspace, project.owner, "scylla-bot", &token)
        .await;
    let id = var("SCYLLA_LIVE_GITLAB_PROJECT_ID");
    let name = var("SCYLLA_LIVE_GITLAB_PROJECT");
    link(&app, &mut project, connection, &id, &name).await;

    // when
    let job = run_on_main(&app, &project).await;
    let report = app
        .wait_for_report(project.owner, &job, ReportState::Sent)
        .await;

    // then
    assert_eq!(report.sent_state, "pending");
}
