//! The tests of the GitHub check runs and their re-run: the check run of a run with its states and
//! its output, the new runs that a person asks for on GitHub, and the check run of a run after an
//! uninstall and a new install of the App.

mod reinstall;
mod reruns;
mod runs;

use scylla_core::api::ServeOptions;
use scylla_core::infra::vcs::sign_github;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::git_ref::Kind;
use scylla_proto::pipelines::v1::CheckoutSettings;
use scylla_testkit::{exec_node, text, FakeProvider, LinkRow, TestApp, TEST_APP_WEBHOOK_SECRET};
use serde_json::{json, Value};
use sqlx::PgPool;

/// A commit: 40 hexadecimal characters.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// The token of the installation 42 of the test GitHub App.
const GITHUB_TOKEN: &str = "ghs_checks";

/// The path of the check runs of `octo/app`.
const CHECK_RUNS: &str = "/repos/octo/app/check-runs";

/// The path of the check run 99, which the fake gives to each new check run.
const CHECK_RUN: &str = "/repos/octo/app/check-runs/99";

/// The page of the check run 99.
const CHECK_PAGE: &str = "https://github.com/octo/app/runs/99";

/// A linked project of a test.
struct Linked {
    /// The owner of the workspace.
    owner: ActorId,
    /// The workspace.
    workspace: WorkspaceId,
    /// The project.
    project: String,
    /// The pipeline `build`, with the nodes `build` and `test`, and no checkout.
    pipeline: String,
}

/// Starts a server that calls a fake GitHub with the test GitHub App: the installation 42 gets a
/// token, `main` is the commit `SHA`, a new check run gets the id 99, and a change of the check
/// run 99 works.
async fn app_with(pool: PgPool, fake: &FakeProvider) -> TestApp {
    let token = json!({ "token": GITHUB_TOKEN, "expires_at": "2099-01-01T00:00:00Z" });
    let path = "/app/installations/42/access_tokens";
    fake.respond("POST", path, 201, token, &[]);
    let commit = json!({
        "sha": SHA,
        "commit": { "message": "Fix the build" },
        "author": { "login": "octocat", "avatar_url": null },
    });
    let path = "/repos/octo/app/commits/heads/main";
    fake.respond("GET", path, 200, commit, &[]);
    let check = json!({ "id": 99, "html_url": CHECK_PAGE });
    fake.respond("POST", CHECK_RUNS, 201, check.clone(), &[]);
    fake.respond("PATCH", CHECK_RUN, 200, check, &[]);
    let options = ServeOptions {
        vcs: fake.options(true),
        ..ServeOptions::default()
    };
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// Makes a project with the pipeline `build`, linked to `octo/app` (id 7) through the
/// installation 42, with the default report mode.
async fn linked(app: &TestApp) -> Linked {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let nodes = vec![exec_node("build", &[]), exec_node("test", &["build"])];
    let checkout = CheckoutSettings {
        enabled: false,
        depth: 1,
        submodules: false,
        lfs: false,
    };
    let pipeline = app
        .create_pipeline_with_checkout(owner, &project, nodes, checkout)
        .await
        .unwrap();
    let connection = app
        .insert_github_connection(workspace, owner, 42, "octo")
        .await;
    let link = LinkRow {
        project: &project,
        connection: &connection,
        external_id: "7",
        full_name: "octo/app",
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;
    Linked {
        owner,
        workspace,
        project,
        pipeline: text(&pipeline.id).to_owned(),
    }
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

/// Returns a GitHub delivery of `check_run` or `check_suite` with the action `rerequested` on a
/// commit of `octo/app`, from the installation 42. `check` is the external id of a check run.
fn rerequested(event: &str, sha: &str, check: Option<&str>) -> Value {
    let mut body = json!({
        "action": "rerequested",
        "installation": { "id": 42 },
        "repository": { "id": 7, "full_name": "octo/app", "html_url": "https://github.com/octo/app" },
        "sender": { "login": "eve" },
    });
    body[event] = json!({ "head_sha": sha, "external_id": check });
    body
}

/// Posts a GitHub delivery to the route of the App of the settings, signed with its secret, and
/// waits for its work. Returns the status of the answer.
async fn post_github(app: &TestApp, event: &str, delivery: &str, body: &Value) -> u16 {
    let body = body.to_string();
    let signature = sign_github(TEST_APP_WEBHOOK_SECRET.as_bytes(), body.as_bytes());
    let response = reqwest::Client::new()
        .post(format!("http://{}/integrations/github/webhook", app.addr))
        .header("x-hub-signature-256", signature)
        .header("x-github-event", event)
        .header("x-github-delivery", delivery)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    if status == 202 {
        app.wait_for_delivery(delivery).await;
    }
    status
}
