//! The tests of the commit context of the runs: a run on a branch, a tag or a SHA, the variables
//! of the commit at the dispatch, the runs from a fork, the new runs and the job lists with
//! filters.

mod lists;
mod live;
mod refs;
mod runs;

use scylla_core::api::ServeOptions;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::Node;
use scylla_testkit::{text, FakeProvider, LinkRow, TestApp};
use serde_json::json;
use sqlx::PgPool;

/// A commit: 40 hexadecimal characters.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// The token of the GitLab service account of the tests.
const GITLAB_TOKEN: &str = "glpat-test";

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

/// Makes a project with one pipeline of these nodes. On `github`, the project is linked to
/// `octo/app` (id 7) through the installation 42. On `gitlab`, it is linked to `group/app` (id
/// 7) through a service account. Else it has no link. The reports of the link are off, so no
/// commit status changes the connection before a dispatch.
async fn linked(app: &TestApp, provider: &str, nodes: Vec<Node>) -> Linked {
    // Make the project and its pipeline.
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
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
        _ => {
            return Linked {
                owner,
                workspace,
                project,
                pipeline,
            }
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
    app.set_link_reports(&project, false).await;
    Linked {
        owner,
        workspace,
        project,
        pipeline,
    }
}

/// Programs a fake GitHub: the installation 42 gets the token `ghs_test`, and
/// `GET /repos/octo/app/commits/<at>` gives the commit `sha` of `octocat`.
fn github_commit(fake: &FakeProvider, at: &str, sha: &str) {
    let token = json!({ "token": "ghs_test", "expires_at": "2099-01-01T00:00:00Z" });
    fake.respond(
        "POST",
        "/app/installations/42/access_tokens",
        201,
        token,
        &[],
    );
    let commit = json!({
        "sha": sha,
        "commit": { "message": "Fix the build\n\nThe long text." },
        "author": { "login": "octocat", "avatar_url": "https://avatars.example/u/1" },
    });
    let path = format!("/repos/octo/app/commits/{at}");
    fake.respond("GET", &path, 200, commit, &[]);
}
