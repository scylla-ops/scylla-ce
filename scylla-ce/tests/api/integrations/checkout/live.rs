//! The tests of a checkout token on a real repository on GitHub.com and GitLab.com: `git
//! ls-remote` reads the repository with the header of the dispatch. They are ignored: they read
//! the accounts of the `SCYLLA_LIVE_*` variables, the dev GitHub App of the
//! `SCYLLA_GITHUB_APP_*` variables, and they need git.

use std::process::Command;
use std::time::Duration;

use scylla_core::api::ServeOptions;
use scylla_core::infra::vcs::{GitHubAppSettings, Hidden, VcsOptions};
use scylla_proto::agents::v1::DispatchCheckout;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_testkit::{exec_node, text, LinkRow, TestApp};
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

/// Returns `true` when `git ls-remote` reads the commit of a checkout with its header only.
fn reads(checkout: &DispatchCheckout) -> bool {
    let setting = format!(
        "http.{}.extraHeader={}",
        checkout.clone_url, checkout.auth_header
    );
    let output = Command::new("git")
        .args(["-c", "credential.helper=", "-c", &setting, "ls-remote"])
        .arg(&checkout.clone_url)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git runs");
    output.status.success() && String::from_utf8_lossy(&output.stdout).contains(&checkout.sha)
}

/// Links a new project to a repository of a connection, runs its pipeline on `main`, and returns
/// the checkout of the dispatch with the job and its owner.
async fn checkout_of(
    app: &TestApp,
    provider: &str,
) -> (
    DispatchCheckout,
    String,
    scylla_domain::identity::actors::ActorId,
) {
    // Make the project and its connection.
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = vec![exec_node("build", &[])];
    let pipeline = app.create_pipeline(owner, &project, nodes).await.unwrap();
    let (connection, external_id, full_name) = match provider {
        "github" => {
            let installation = var("SCYLLA_LIVE_GITHUB_INSTALLATION").parse().unwrap();
            let repository = var("SCYLLA_LIVE_GITHUB_REPOSITORY");
            let account = repository.split('/').next().unwrap_or_default().to_owned();
            let connection = app.insert_github_connection(workspace, owner, installation, &account);
            let id = var("SCYLLA_LIVE_GITHUB_REPOSITORY_ID");
            (connection.await, id, repository)
        }
        _ => {
            let token = var("SCYLLA_LIVE_GITLAB_TOKEN");
            let connection = app.insert_gitlab_connection(workspace, owner, "scylla-bot", &token);
            let id = var("SCYLLA_LIVE_GITLAB_PROJECT_ID");
            (connection.await, id, var("SCYLLA_LIVE_GITLAB_PROJECT"))
        }
    };
    let link = LinkRow {
        project: &project,
        connection: &connection,
        external_id: &external_id,
        full_name: &full_name,
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;

    // Run the pipeline on main, and take the dispatch.
    let main = Kind::Branch("main".to_owned());
    let job = app.run_pipeline_on(owner, text(&pipeline.id), main).await;
    let job = text(&job.unwrap().id).to_owned();
    let (dispatch, _up) = app.first_dispatch(owner, workspace).await;
    (dispatch.checkout.expect("a checkout"), job, owner)
}

/// The installation token of a checkout reads the test repository of the dev GitHub App.
#[ignore = "needs the dev GitHub App, SCYLLA_LIVE_GITHUB_* and git"]
#[sqlx::test(migrations = false)]
async fn a_live_github_checkout_token_reads_the_repository(pool: PgPool) {
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

    // when
    let (checkout, _, _) = checkout_of(&app, "github").await;

    // then
    assert!(reads(&checkout), "git did not read {}", checkout.clone_url);
}

/// The deploy token of a checkout reads the test project of the GitLab service account, and
/// stops at the end of the job.
#[ignore = "needs the GitLab service account, SCYLLA_LIVE_GITLAB_* and git"]
#[sqlx::test(migrations = false)]
async fn a_live_gitlab_deploy_token_ends_with_the_job(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let (checkout, job, owner) = checkout_of(&app, "gitlab").await;
    let read = reads(&checkout);

    // when
    app.cancel_job(owner, &job).await.unwrap();
    for _ in 0..100 {
        if app.checkout_tokens().await.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // then
    assert!(read, "git did not read {}", checkout.clone_url);
    assert!(app.checkout_tokens().await.is_empty());
    assert!(!reads(&checkout), "the deleted token still reads");
}
