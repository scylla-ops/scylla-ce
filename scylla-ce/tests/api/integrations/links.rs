//! The tests of the repository links: the link of a GitHub repository and of a GitLab project
//! with its webhook, the changes, the unlink, the branches and the tags, and the access.

mod access;
mod changes;
mod github;
mod gitlab;
mod live;
mod refs;

use scylla_core::api::ServeOptions;
use scylla_domain::identity::actors::ActorId;
use scylla_testkit::{text, FakeProvider, LinkRow, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

/// The token of the GitLab service account of the tests.
const GITLAB_TOKEN: &str = "glpat-links";

/// A project of a test, with a connection of its workspace.
struct Project {
    /// The owner of the workspace.
    owner: ActorId,
    /// The project.
    id: String,
    /// The connection: the installation 42 of `octo` on GitHub, or a service account on GitLab.
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

/// Makes a project and a connection of the provider (`github` or `gitlab`) in the workspace of a
/// new owner.
async fn project(app: &TestApp, provider: &str) -> Project {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "API").await.unwrap();
    let connection = match provider {
        "github" => {
            app.insert_github_connection(workspace, owner, 42, "octo")
                .await
        }
        _ => {
            app.insert_gitlab_connection(workspace, owner, "scylla-bot", GITLAB_TOKEN)
                .await
        }
    };
    Project {
        owner,
        id: text(&project.id).to_owned(),
        connection,
    }
}

/// Links a project to the repository 7 of its connection with the rows of the fixtures.
async fn insert_link(app: &TestApp, project: &Project, full_name: &str) {
    let link = LinkRow {
        project: &project.id,
        connection: &project.connection,
        external_id: "7",
        full_name,
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;
}

/// Checks that a call failed with `FAILED_PRECONDITION` and this reason.
fn assert_reason<T: std::fmt::Debug>(result: Result<T, Status>, reason: &str) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo");
    assert_eq!(info.reason, reason, "{status:?}");
}
