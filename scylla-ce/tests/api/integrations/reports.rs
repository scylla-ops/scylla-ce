//! The tests of the reports of the runs on the commits: the commit statuses on GitHub and
//! GitLab, the failed calls, the access to the reports, the health of the connections, and the
//! live providers.

mod access;
mod failures;
mod health;
mod live;
mod statuses;

use scylla_core::api::ServeOptions;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_testkit::{exec_node, text, FakeProvider, LinkRow, TestApp};
use serde_json::json;
use sqlx::PgPool;

/// A commit: 40 hexadecimal characters.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// The token of the GitLab service account of the tests.
const GITLAB_TOKEN: &str = "glpat-reports";

/// The token of the installation 42 of the test GitHub App.
const GITHUB_TOKEN: &str = "ghs_reports";

/// The path of the commit statuses of the commit on GitHub.
const GITHUB_STATUSES: &str = "/repos/octo/app/statuses/0123456789abcdef0123456789abcdef01234567";

/// The path of the commit statuses of the commit on GitLab.
const GITLAB_STATUSES: &str =
    "/api/v4/projects/7/statuses/0123456789abcdef0123456789abcdef01234567";

/// A linked project of a test.
struct Linked {
    /// The owner of the workspace.
    owner: ActorId,
    /// The workspace.
    workspace: WorkspaceId,
    /// The project.
    project: String,
    /// The pipeline, named `build`.
    pipeline: String,
    /// The connection, or an empty text for a project with no link.
    connection: String,
}

/// Starts a server that calls a fake provider for GitHub and GitLab, with the test GitHub App.
async fn app_with(pool: PgPool, fake: &FakeProvider) -> TestApp {
    let options = ServeOptions {
        vcs: fake.options(true),
        ..ServeOptions::default()
    };
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// Makes a project with the pipeline `build`. On `github`, the project is linked to `octo/app`
/// (id 7) through the installation 42. On `gitlab`, it is linked to `group/app` (id 7) through a
/// service account. Else it has no link.
async fn linked(app: &TestApp, provider: &str) -> Linked {
    // Make the project and its pipeline.
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = vec![exec_node("build", &[])];
    let pipeline = app.create_pipeline(owner, &project, nodes).await.unwrap();
    let pipeline = text(&pipeline.id).to_owned();

    // Link the repository through a connection of the provider.
    let (connection, full_name) = match provider {
        "github" => (
            app.insert_github_connection(workspace, owner, 42, "octo")
                .await,
            "octo/app",
        ),
        "gitlab" => (
            app.insert_gitlab_connection(workspace, owner, "scylla-bot", GITLAB_TOKEN)
                .await,
            "group/app",
        ),
        _ => (String::new(), ""),
    };
    if !connection.is_empty() {
        let link = LinkRow {
            project: &project,
            connection: &connection,
            external_id: "7",
            full_name,
            default_branch: "main",
        };
        app.insert_repository_link(&link).await;
    }
    Linked {
        owner,
        workspace,
        project,
        pipeline,
        connection,
    }
}

/// Programs a fake GitHub: the installation 42 gets a token, `main` is the commit `SHA`, and
/// each commit status gets `status`.
fn github(fake: &FakeProvider, status: u16) {
    let token = json!({ "token": GITHUB_TOKEN, "expires_at": "2099-01-01T00:00:00Z" });
    fake.respond(
        "POST",
        "/app/installations/42/access_tokens",
        201,
        token,
        &[],
    );
    let commit = json!({
        "sha": SHA,
        "commit": { "message": "Fix the build" },
        "author": { "login": "octocat", "avatar_url": null },
    });
    fake.respond(
        "GET",
        "/repos/octo/app/commits/heads/main",
        200,
        commit,
        &[],
    );
    let answer = json!({ "id": 1, "state": "pending" });
    fake.respond("POST", GITHUB_STATUSES, status, answer, &[]);
}

/// Programs a fake GitLab: `main` is the commit `SHA`, and each commit status gets the GitLab
/// pipeline 77.
fn gitlab(fake: &FakeProvider) {
    let commit = json!({ "id": SHA, "message": "Fix the build" });
    let path = "/api/v4/projects/7/repository/commits/main";
    fake.respond("GET", path, 200, commit, &[]);
    let answer = json!({ "id": 5, "status": "pending", "pipeline_id": 77 });
    fake.respond("POST", GITLAB_STATUSES, 201, answer, &[]);
}

/// Runs the pipeline of a linked project on `main`, and returns the job.
async fn run_on_main(app: &TestApp, linked: &Linked) -> String {
    let main = Kind::Branch("main".to_owned());
    let job = app
        .run_pipeline_on(linked.owner, &linked.pipeline, main)
        .await
        .unwrap();
    text(&job.id).to_owned()
}
