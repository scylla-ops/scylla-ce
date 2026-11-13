//! The tests of the webhooks of the providers: the signatures, the deliveries kept one time, the
//! arrival of a webhook, the changes of the installations of the GitHub App, and the test of the
//! webhook of a link.

mod github;
mod gitlab;
mod installations;
mod test_delivery;

use std::time::{SystemTime, UNIX_EPOCH};

use scylla_core::api::ServeOptions;
use scylla_core::infra::vcs::{sign_github, sign_gitlab, Hidden};
use scylla_domain::identity::actors::ActorId;
use scylla_testkit::{text, FakeProvider, LinkRow, TestApp, TEST_APP_WEBHOOK_SECRET};
use serde_json::{json, Value};
use sqlx::PgPool;

/// A commit: 40 hexadecimal characters.
const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

/// The token of the GitLab service account of the tests.
const GITLAB_TOKEN: &str = "glpat-events";

/// The signing token of the GitLab webhook of the tests: `whsec_` and a key in base64.
const SIGNING_TOKEN: &str = "whsec_dGhlIHNpZ25pbmcga2V5IG9mIHRoZSB0ZXN0cw==";

/// The route of the webhooks of the App of the settings.
const GITHUB_ROUTE: &str = "/integrations/github/webhook";

/// A linked project of a test.
struct Linked {
    /// The owner of the workspace.
    owner: ActorId,
    /// The project.
    project: String,
    /// The connection.
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

/// Makes a project linked to the repository 7 in webhook mode. On `github`, it is `octo/app` of
/// the installation 42. On `gitlab`, it is `group/app`, with the webhook 9 and the signing token
/// of the tests.
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
            app.insert_gitlab_connection(workspace, owner, "scylla-bot", GITLAB_TOKEN)
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
    if provider == "gitlab" {
        app.set_link_signing_token(&project, "9", SIGNING_TOKEN)
            .await;
    }
    Linked {
        owner,
        project,
        connection,
    }
}

/// Posts a GitHub delivery to a route, signed with a secret. Returns the status and the body.
async fn post_github(
    app: &TestApp,
    route: &str,
    event: &str,
    delivery: &str,
    body: &Value,
    secret: &str,
) -> (u16, Value) {
    let body = body.to_string();
    let signature = sign_github(secret.as_bytes(), body.as_bytes());
    let response = reqwest::Client::new()
        .post(format!("http://{}{route}", app.addr))
        .header("x-hub-signature-256", signature)
        .header("x-github-event", event)
        .header("x-github-delivery", delivery)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

/// Posts a GitHub delivery to the route of the App of the settings, with its secret.
async fn post_app(app: &TestApp, event: &str, delivery: &str, body: &Value) -> (u16, Value) {
    post_github(
        app,
        GITHUB_ROUTE,
        event,
        delivery,
        body,
        TEST_APP_WEBHOOK_SECRET,
    )
    .await
}

/// Posts a GitLab delivery to the webhook of a project, signed with a signing token at a time
/// `age` seconds before now. Returns the status and the body.
async fn post_gitlab(
    app: &TestApp,
    project: &str,
    key: &str,
    body: &Value,
    signing_token: &str,
    age: i64,
) -> (u16, Value) {
    let body = body.to_string();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let timestamp = (i64::try_from(now.as_secs()).unwrap() - age).to_string();
    let token = Hidden::new(signing_token);
    let signature = sign_gitlab(&token, key, &timestamp, body.as_bytes());
    let response = reqwest::Client::new()
        .post(format!(
            "http://{}/integrations/gitlab/{project}/webhook",
            app.addr
        ))
        .header("webhook-id", key)
        .header("webhook-timestamp", timestamp)
        .header("webhook-signature", signature)
        .header("x-gitlab-event", "Push Hook")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

/// Returns a GitHub push of a commit to a branch of `octo/app` (id 7), from the installation
/// 42.
fn github_push(sha: &str, branch: &str) -> Value {
    json!({
        "ref": format!("refs/heads/{branch}"),
        "after": sha,
        "deleted": false,
        "head_commit": { "id": sha, "message": "Fix the build", "author": { "username": "octocat" } },
        "repository": { "id": 7, "full_name": "octo/app", "html_url": "https://github.com/octo/app" },
        "installation": { "id": 42 },
        "sender": { "login": "octocat", "avatar_url": null },
    })
}

/// Returns a GitLab push of a commit to a branch of `group/app` (id 7).
fn gitlab_push(sha: &str, branch: &str) -> Value {
    json!({
        "object_kind": "push",
        "ref": format!("refs/heads/{branch}"),
        "after": sha,
        "checkout_sha": sha,
        "user_username": "dev",
        "project": { "id": 7, "path_with_namespace": "group/app", "web_url": "https://gitlab.com/group/app" },
        "commits": [{ "id": sha, "message": "Fix the build" }],
    })
}
