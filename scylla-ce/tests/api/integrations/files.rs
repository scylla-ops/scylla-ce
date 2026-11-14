//! The tests of the pipelines from the files of a repository: the source of the pipelines of a
//! project, the sync with the default branch, the read of the files at a commit, the read only
//! pipelines, the triggers of a file, and the runs of the files at a commit.

mod events;
mod gitlab;
mod live;
mod reads;
mod rules;
mod runs;
mod source;

use scylla_core::api::ServeOptions;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::integrations::v1::PipelineSource;
use scylla_proto::pipelines::v1::{Job, Pipeline};
use scylla_testkit::{exec_node, text, FakeProvider, LinkRow, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

/// The head of the default branch.
const MAIN_SHA: &str = "1111111111111111111111111111111111111111";

/// The head of the default branch after a push.
const NEXT_SHA: &str = "2222222222222222222222222222222222222222";

/// The head of a branch of a pull request.
const FEATURE_SHA: &str = "3333333333333333333333333333333333333333";

/// The path of the file of the pipeline `build`.
const BUILD_PATH: &str = ".scylla/pipelines/build.toml";

/// The file of the pipeline `build`: four nodes, and a push to `main` or `release/*`.
const BUILD: &str = r#"version = 1
name = "Build"

[on.push]
branches = ["main", "release/*"]

[[nodes]]
id = "fetch"
exec = ["make", "fetch"]

[[nodes]]
id = "compile"
needs = ["fetch"]
exec = ["make"]

[[nodes]]
id = "test"
needs = ["compile"]
exec = ["make", "test"]

[[nodes]]
id = "package"
needs = ["test"]
exec = ["make", "package"]
"#;

/// The path of the file of the pipeline `deploy`.
const DEPLOY_PATH: &str = ".scylla/pipelines/deploy.yaml";

/// The file of the pipeline `deploy`: one node and no `on` table.
const DEPLOY: &str = "version: 1\nname: Deploy\nnodes:\n  - id: ship\n    exec: [make, ship]\n";

/// The path of the file of the pipeline `lint`.
const LINT_PATH: &str = ".scylla/pipelines/lint.json";

/// The file of the pipeline `lint`: a node that needs a node that does not exist.
const LINT: &str = r#"{
  "version": 1,
  "nodes": [
    { "id": "lint", "needs": ["tset"], "exec": ["make", "lint"] }
  ]
}
"#;

/// The file of the pipeline `build` with a fifth node, `bench`, as a pull request adds it.
fn bench() -> String {
    format!("{BUILD}\n[[nodes]]\nid = \"bench\"\nneeds = [\"test\"]\nexec = [\"make\"]\n")
}

/// A linked project of a test.
struct Linked {
    /// The owner of the workspace.
    owner: ActorId,
    /// The workspace.
    workspace: WorkspaceId,
    /// The project.
    project: String,
    /// The editor pipeline of the project.
    editor: String,
}

/// Starts a server that calls a fake provider for GitHub and GitLab, with the test GitHub App.
async fn app_with(pool: PgPool, fake: &FakeProvider) -> TestApp {
    let options = ServeOptions {
        vcs: fake.options(true),
        ..ServeOptions::default()
    };
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// Makes a project with one editor pipeline. On `github`, the project is linked to `octo/app`
/// (id 7) through the installation 42. On `gitlab`, it is linked to `group/app` (id 7) through a
/// service account. Else it has no link. The default branch is `main`, and the link reports
/// nothing.
async fn linked(app: &TestApp, provider: &str) -> Linked {
    // Make the project and its editor pipeline.
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = vec![exec_node("build", &[])];
    let editor = app.create_pipeline(owner, &project, nodes).await.unwrap();
    let editor = text(&editor.id).to_owned();

    // Link the repository through a connection of the provider.
    let (connection, full_name) = match provider {
        "github" => {
            let connection = app.insert_github_connection(workspace, owner, 42, "octo");
            (connection.await, "octo/app")
        }
        "gitlab" => {
            let token = "glpat-test";
            let connection = app.insert_gitlab_connection(workspace, owner, "bot", token);
            (connection.await, "group/app")
        }
        _ => {
            return Linked {
                owner,
                workspace,
                project,
                editor,
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
        editor,
    }
}

/// A project of files of a test, linked to `octo/app` on GitHub.
struct FilesProject {
    /// The linked project.
    linked: Linked,
    /// The file pipeline `build`.
    build: String,
}

/// Makes a project linked on GitHub with the reports on or off, and moves it to the files of the
/// default branch, which must have the file of `build`.
async fn files_project(app: &TestApp, reports: bool) -> FilesProject {
    let linked = linked(app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    app.set_link_reports(project, reports).await;
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let build = text(&by_key(&pipelines(app, owner, project).await, "build").id).to_owned();
    FilesProject { linked, build }
}

/// Returns the jobs of the first page of a pipeline, newest first.
async fn jobs_of(app: &TestApp, actor: ActorId, pipeline: &str) -> Vec<Job> {
    let page = app.list_pipeline_jobs(actor, pipeline, 0, "").await;
    page.unwrap().jobs
}

/// Returns the pipelines of the first page of a project.
async fn pipelines(app: &TestApp, actor: ActorId, project: &str) -> Vec<Pipeline> {
    let page = app.list_project_pipelines(actor, project, 0, "").await;
    page.unwrap().pipelines
}

/// Returns the pipeline of a file key in a list.
fn by_key<'a>(pipelines: &'a [Pipeline], key: &str) -> &'a Pipeline {
    pipelines
        .iter()
        .find(|pipeline| pipeline.file_key == key)
        .unwrap_or_else(|| panic!("no pipeline with the key {key}"))
}

/// Checks that a call fails with `FAILED_PRECONDITION` and a reason.
fn assert_reason<T: std::fmt::Debug>(result: Result<T, Status>, reason: &str) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    let info = status.get_details_error_info().expect("ErrorInfo");
    assert_eq!(info.reason, reason, "{status:?}");
}
