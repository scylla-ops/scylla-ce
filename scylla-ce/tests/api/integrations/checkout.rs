//! The tests of the checkout of a job on an agent: the placement on the agents with `checkout`,
//! the short-lived token of the dispatch, the events and the log lines of the checkout, the
//! GitLab deploy tokens, and the checkout settings of an editor pipeline.

mod dispatch;
mod events;
mod gitlab;
mod live;
mod settings;

use std::time::Duration;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use scylla_core::api::ServeOptions;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::{CheckoutSettings, Job};
use scylla_testkit::{exec_node, text, FakeProvider, LinkRow, TestApp};
use serde_json::json;
use sqlx::PgPool;

/// A commit: 40 hexadecimal characters.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// The token of the GitLab service account of the tests.
const GITLAB_TOKEN: &str = "glpat-test";

/// The installation token that the fake GitHub gives.
const GITHUB_TOKEN: &str = "ghs_checkout";

/// The deploy token that the fake GitLab gives.
const DEPLOY_TOKEN: &str = "gldt-checkout";

/// A linked project of a test.
struct Linked {
    /// The owner of the workspace.
    owner: ActorId,
    /// The workspace.
    workspace: WorkspaceId,
    /// The project.
    project: String,
    /// The pipeline.
    pipeline: String,
}

/// Starts a server that calls a fake provider for GitHub and GitLab, with the test GitHub App.
async fn app_with(pool: PgPool, fake: &FakeProvider) -> TestApp {
    let options = ServeOptions {
        vcs: fake.options(true),
        ..ServeOptions::default()
    };
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// Makes a project with a pipeline of one node and these checkout settings (`None` gives the
/// default). On `github`, the project is linked to `octo/app` (id 7) through the installation 42.
/// On `gitlab`, it is linked to `group/app` (id 7) through a service account.
async fn linked(app: &TestApp, provider: &str, checkout: Option<CheckoutSettings>) -> Linked {
    // Make the project and its pipeline.
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = vec![exec_node("build", &[])];
    let pipeline = match checkout {
        Some(checkout) => {
            let create = app.create_pipeline_with_checkout(owner, &project, nodes, checkout);
            create.await
        }
        None => app.create_pipeline(owner, &project, nodes).await,
    };
    let pipeline = text(&pipeline.unwrap().id).to_owned();

    // Link the repository through a connection of the provider.
    let (connection, full_name) = match provider {
        "github" => {
            let github = app.insert_github_connection(workspace, owner, 42, "octo");
            (github.await, "octo/app")
        }
        _ => {
            let gitlab = app.insert_gitlab_connection(workspace, owner, "scylla-bot", GITLAB_TOKEN);
            (gitlab.await, "group/app")
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
    Linked {
        owner,
        workspace,
        project,
        pipeline,
    }
}

/// Programs a fake GitHub: the installation 42 gets the token `GITHUB_TOKEN`, and the branch
/// `main` of `octo/app` is at `SHA`.
fn github(fake: &FakeProvider) {
    let token = json!({ "token": GITHUB_TOKEN, "expires_at": "2099-01-01T00:00:00Z" });
    let path = "/app/installations/42/access_tokens";
    fake.respond("POST", path, 201, token, &[]);
    let commit = json!({
        "sha": SHA,
        "commit": { "message": "Fix the build" },
        "author": { "login": "octocat", "avatar_url": "https://avatars.example/u/1" },
    });
    fake.respond(
        "GET",
        "/repos/octo/app/commits/heads/main",
        200,
        commit,
        &[],
    );
}

/// Programs a fake GitLab: the branch `main` of the project 7 is at `SHA`, a new deploy token is
/// the token 99, and its delete works.
fn gitlab(fake: &FakeProvider) {
    let commit = json!({ "id": SHA, "message": "Fix the build" });
    let path = "/api/v4/projects/7/repository/commits/main";
    fake.respond("GET", path, 200, commit, &[]);
    let token = json!({
        "id": 99,
        "name": "scylla-job",
        "username": "gitlab+deploy-token-99",
        "token": DEPLOY_TOKEN,
        "scopes": ["read_repository"],
    });
    fake.respond("POST", "/api/v4/projects/7/deploy_tokens", 201, token, &[]);
    let path = "/api/v4/projects/7/deploy_tokens/99";
    fake.respond("DELETE", path, 204, serde_json::Value::Null, &[]);
}

/// Returns the header of a basic authentication.
fn basic(user: &str, token: &str) -> String {
    format!(
        "Authorization: Basic {}",
        STANDARD.encode(format!("{user}:{token}"))
    )
}

/// Reads a job as its owner until `done` gives `true`, for at most 10 seconds.
async fn job_when(app: &TestApp, owner: ActorId, job: &str, done: impl Fn(&Job) -> bool) -> Job {
    for _ in 0..200 {
        let read = app.get_job(owner, job).await.unwrap();
        if done(&read) {
            return read;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the job {job} did not reach the state");
}
