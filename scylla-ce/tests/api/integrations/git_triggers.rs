//! The tests of the runs of the push, pull request and tag events: the Git triggers that match an
//! event, one run for each pipeline and commit, the runs from a fork, the GitLab merge requests,
//! and a new head that a poll finds.

mod forks;
mod gitlab;
mod polling;
mod pushes;

use std::time::{SystemTime, UNIX_EPOCH};

use scylla_core::api::ServeOptions;
use scylla_core::infra::vcs::{sign_github, sign_gitlab, Hidden};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::Job;
use scylla_testkit::{
    exec_node, text, DeliveryRow, FakeProvider, LinkRow, TestApp, TEST_APP_WEBHOOK_SECRET,
};
use serde_json::{json, Value};
use sqlx::PgPool;

/// A commit: 40 hexadecimal characters.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// Another commit.
const OTHER_SHA: &str = "89abcdef0123456789abcdef0123456789abcdef";

/// The signing token of the GitLab webhook of the tests: `whsec_` and a key in base64.
const SIGNING_TOKEN: &str = "whsec_dGhlIHNpZ25pbmcga2V5IG9mIHRoZSB0ZXN0cw==";

/// A linked project of a test.
struct Linked {
    /// The owner of the workspace.
    owner: ActorId,
    /// The workspace.
    workspace: WorkspaceId,
    /// The project.
    project: String,
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

/// Makes a project linked to the repository 7 in webhook mode, with the reports off. On
/// `github`, it is `octo/app` of the installation 42. On `gitlab`, it is `group/app`, with the
/// webhook 9 and the signing token of the tests.
async fn linked(app: &TestApp, provider: &str) -> Linked {
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "API").await.unwrap();
    let project = text(&project.id).to_owned();
    let (connection, full_name) = match provider {
        "github" => (
            app.insert_github_connection(workspace, owner, 42, "octo")
                .await,
            "octo/app",
        ),
        _ => (
            app.insert_gitlab_connection(workspace, owner, "scylla-bot", "glpat-runs")
                .await,
            "group/app",
        ),
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
    if provider == "gitlab" {
        app.set_link_signing_token(&project, "9", SIGNING_TOKEN)
            .await;
    }
    Linked {
        owner,
        workspace,
        project,
    }
}

/// Makes a pipeline of one node in the linked project, with one enabled Git trigger for each
/// `(kind, rules)`. Returns the pipeline and its triggers.
async fn pipeline_with(
    app: &TestApp,
    linked: &Linked,
    triggers: &[(&str, Value)],
) -> (String, Vec<String>) {
    let nodes = vec![exec_node("build", &[])];
    let pipeline = app.create_pipeline(linked.owner, &linked.project, nodes);
    let pipeline = text(&pipeline.await.unwrap().id).to_owned();
    let mut ids = Vec::new();
    for (kind, rules) in triggers {
        let name = format!("on {kind}");
        let id = app.insert_git_trigger(&pipeline, &name, kind, rules.clone());
        ids.push(id.await);
    }
    (pipeline, ids)
}

/// Returns the jobs of a pipeline, newest first.
async fn jobs_of(app: &TestApp, linked: &Linked, pipeline: &str) -> Vec<Job> {
    let page = app.list_pipeline_jobs(linked.owner, pipeline, 0, "");
    page.await.unwrap().jobs
}

/// Posts a signed GitHub delivery of the App of the settings, checks that it gets 202, and waits
/// for its work.
async fn deliver_github(app: &TestApp, event: &str, key: &str, body: &Value) -> DeliveryRow {
    let body = body.to_string();
    let signature = sign_github(TEST_APP_WEBHOOK_SECRET.as_bytes(), body.as_bytes());
    let response = reqwest::Client::new()
        .post(format!("http://{}/integrations/github/webhook", app.addr))
        .header("x-hub-signature-256", signature)
        .header("x-github-event", event)
        .header("x-github-delivery", key)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 202);
    app.wait_for_delivery(key).await
}

/// Posts a signed GitLab delivery to the webhook of a project, checks that it gets 202, and waits
/// for its work.
async fn deliver_gitlab(app: &TestApp, project: &str, key: &str, body: &Value) -> DeliveryRow {
    let body = body.to_string();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let timestamp = now.as_secs().to_string();
    let signature = sign_gitlab(
        &Hidden::new(SIGNING_TOKEN),
        key,
        &timestamp,
        body.as_bytes(),
    );
    let response = reqwest::Client::new()
        .post(format!(
            "http://{}/integrations/gitlab/{project}/webhook",
            app.addr
        ))
        .header("webhook-id", key)
        .header("webhook-timestamp", timestamp)
        .header("webhook-signature", signature)
        .header("x-gitlab-event", "Merge Request Hook")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 202);
    app.wait_for_delivery(key).await
}

/// Returns a GitHub push of a commit to a ref of `octo/app` (id 7), from the installation 42.
fn github_push(git_ref: &str, sha: &str, message: &str) -> Value {
    json!({
        "ref": git_ref,
        "after": sha,
        "deleted": false,
        "head_commit": { "id": sha, "message": message, "author": { "username": "octocat" } },
        "repository": { "id": 7, "full_name": "octo/app", "html_url": "https://github.com/octo/app" },
        "installation": { "id": 42 },
        "sender": { "login": "octocat", "avatar_url": null },
    })
}

/// Returns a GitHub action on the pull request `number` of `octo/app` (id 7) into `main`, from
/// the branch `feature/login` of the repository `head` (`(id, full_name)`) at a commit.
fn github_pull(action: &str, number: u32, sha: &str, head: (u64, &str), draft: bool) -> Value {
    let repo = |(id, name): (u64, &str)| json!({ "id": id, "full_name": name });
    json!({
        "action": action,
        "pull_request": {
            "number": number,
            "title": "Add login",
            "html_url": format!("https://github.com/octo/app/pull/{number}"),
            "draft": draft,
            "head": { "ref": "feature/login", "sha": sha, "repo": repo(head) },
            "base": { "ref": "main", "sha": OTHER_SHA, "repo": repo((7, "octo/app")) },
            "user": { "login": "octocat", "avatar_url": null },
        },
        "repository": { "id": 7, "full_name": "octo/app", "html_url": "https://github.com/octo/app" },
        "installation": { "id": 42 },
        "sender": { "login": "octocat", "avatar_url": null },
    })
}

/// Returns a GitLab action on the merge request `iid` of `group/app` (id 7) into `main`, from the
/// branch `feature` of the project `source` (`(id, path)`), with a head commit.
fn gitlab_merge_request(action: &str, iid: u32, sha: &str, source: (u64, &str)) -> Value {
    json!({
        "object_kind": "merge_request",
        "user": { "username": "dev", "avatar_url": null },
        "project": { "id": 7, "path_with_namespace": "group/app", "web_url": "https://gitlab.com/group/app" },
        "object_attributes": {
            "iid": iid,
            "title": "Add login",
            "url": format!("https://gitlab.com/group/app/-/merge_requests/{iid}"),
            "source_branch": "feature",
            "target_branch": "main",
            "source_project_id": source.0,
            "target_project_id": 7,
            "action": action,
            "draft": false,
            "last_commit": { "id": sha, "message": "Add login" },
            "source": { "id": source.0, "path_with_namespace": source.1, "web_url": "" },
        },
    })
}
