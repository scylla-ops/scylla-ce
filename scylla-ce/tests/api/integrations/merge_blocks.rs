//! The tests of the rules that block a merge: the ruleset of Scylla on GitHub, "Pipelines must
//! succeed" on GitLab, the refusals of the provider, a rule out of sync, the archived pipelines,
//! and the access.

mod access;
mod archived;
mod github;
mod gitlab;
mod live;

use scylla_core::api::ServeOptions;
use scylla_domain::identity::actors::ActorId;
use scylla_testkit::{exec_node, text, FakeProvider, LinkRow, TestApp};
use serde_json::json;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

/// The token of the GitLab service account of the tests.
const GITLAB_TOKEN: &str = "glpat-merge";

/// A linked project of a test, with the pipeline `build`.
struct Linked {
    /// The owner of the workspace.
    owner: ActorId,
    /// The project.
    project: String,
    /// The pipeline `build`.
    pipeline: String,
    /// The connection: the installation 42 of `octo` on GitHub, with `administration: write`, or
    /// a service account on GitLab.
    connection: String,
}

/// Starts the Community edition with the test GitHub App, and with GitHub and GitLab on a fake.
async fn app_with_fake(pool: PgPool) -> (TestApp, FakeProvider) {
    let fake = FakeProvider::start().await;
    let options = ServeOptions {
        vcs: fake.options(true),
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    (app, fake)
}

/// Makes a project with the pipeline `build`, linked to the public repository 7 of a connection
/// of the provider (`github` or `gitlab`): `octo/app` on GitHub, `group/api` on GitLab with the
/// role Maintainer.
async fn linked(app: &TestApp, provider: &str) -> Linked {
    // Make the project and its pipeline.
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "API").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = vec![exec_node("compile", &[])];
    let pipeline = app.create_pipeline(owner, &project, nodes).await.unwrap();

    // Link the repository of a connection.
    let (connection, full_name) = match provider {
        "github" => {
            let connection = app
                .insert_github_connection(workspace, owner, 42, "octo")
                .await;
            let permissions = json!({ "metadata": "read", "administration": "write" });
            app.set_connection_permissions(&connection, permissions)
                .await;
            (connection, "octo/app")
        }
        _ => {
            let connection = app
                .insert_gitlab_connection(workspace, owner, "scylla-bot", GITLAB_TOKEN)
                .await;
            (connection, "group/api")
        }
    };
    let link = LinkRow {
        project: &project,
        connection: &connection,
        external_id: "7",
        full_name,
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;
    if provider == "gitlab" {
        app.set_link_access(&project, true, Some("maintainer"))
            .await;
    }
    Linked {
        owner,
        project,
        pipeline: text(&pipeline.id).to_owned(),
        connection,
    }
}

/// Checks that a call failed with `FAILED_PRECONDITION` and this reason.
fn assert_reason<T: std::fmt::Debug>(result: Result<T, Status>, reason: &str) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo");
    assert_eq!(info.reason, reason, "{status:?}");
}
