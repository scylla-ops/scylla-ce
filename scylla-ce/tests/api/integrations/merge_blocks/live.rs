//! The tests of the merge rules on a real repository of GitHub.com and a real project of
//! GitLab.com. They are ignored: they read the accounts of the `SCYLLA_LIVE_*` variables, and the
//! dev GitHub App of the `SCYLLA_GITHUB_APP_*` variables.

use scylla_core::api::ServeOptions;
use scylla_core::infra::vcs::{GitHubAppSettings, Hidden, VcsOptions};
use scylla_proto::integrations::v1::MergeBlockState;
use scylla_testkit::{exec_node, text, LinkRow, TestApp};
use serde_json::json;
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

/// The dev App makes the ruleset "Scylla required checks" on the test repository, then removes
/// it. The repository is public, or its organization has GitHub Team.
#[ignore = "needs the dev GitHub App and SCYLLA_LIVE_GITHUB_*"]
#[sqlx::test(migrations = false)]
async fn set_merge_block_on_a_live_github_repository(pool: PgPool) {
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
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "Live").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = vec![exec_node("build", &[])];
    let pipeline = app.create_pipeline(owner, &project, nodes).await.unwrap();
    let pipeline = text(&pipeline.id).to_owned();
    let installation = var("SCYLLA_LIVE_GITHUB_INSTALLATION_ID")
        .parse()
        .expect("an id");
    let repository = var("SCYLLA_LIVE_GITHUB_REPOSITORY");
    let account = repository.split('/').next().unwrap_or_default();
    let connection = app
        .insert_github_connection(workspace, owner, installation, account)
        .await;
    let permissions = json!({ "metadata": "read", "administration": "write" });
    app.set_connection_permissions(&connection, permissions)
        .await;
    let link = LinkRow {
        project: &project,
        connection: &connection,
        external_id: &var("SCYLLA_LIVE_GITHUB_REPOSITORY_ID"),
        full_name: &repository,
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;

    // when
    let on = app
        .set_merge_block(owner, &project, Some(&pipeline), true)
        .await;
    let listed = app.list_merge_blocks(owner, &project).await;
    let off = app
        .set_merge_block(owner, &project, Some(&pipeline), false)
        .await;

    // then
    let on = on.unwrap();
    assert_eq!(on.state, MergeBlockState::On as i32, "{on:?}");
    assert_eq!(listed.unwrap()[0].state, MergeBlockState::On as i32);
    assert_eq!(off.unwrap().state, MergeBlockState::Off as i32);
    assert_eq!(app.ruleset_of(&project).await, None);
}

/// The service account turns "Pipelines must succeed" on in the test project, then off. It is
/// Maintainer of the project.
#[ignore = "needs the GitLab service account and SCYLLA_LIVE_GITLAB_*"]
#[sqlx::test(migrations = false)]
async fn set_merge_block_on_a_live_gitlab_project(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "Live").await.unwrap();
    let project = text(&project.id).to_owned();
    let token = var("SCYLLA_LIVE_GITLAB_TOKEN");
    let connection = app
        .insert_gitlab_connection(workspace, owner, "scylla-bot", &token)
        .await;
    let link = LinkRow {
        project: &project,
        connection: &connection,
        external_id: &var("SCYLLA_LIVE_GITLAB_PROJECT_ID"),
        full_name: &var("SCYLLA_LIVE_GITLAB_PROJECT"),
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;
    app.set_link_access(&project, true, Some("maintainer"))
        .await;

    // when
    let on = app.set_merge_block(owner, &project, None, true).await;
    let listed = app.list_merge_blocks(owner, &project).await;
    let off = app.set_merge_block(owner, &project, None, false).await;

    // then
    let on = on.unwrap();
    assert_eq!(on.state, MergeBlockState::On as i32, "{on:?}");
    assert_eq!(listed.unwrap()[0].state, MergeBlockState::On as i32);
    assert_eq!(off.unwrap().state, MergeBlockState::Off as i32);
}
