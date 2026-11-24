//! The tests of the HTTP/JSON gateway: a method in JSON, the errors in JSON, and the secret of
//! an API token sent directly.

use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::auth::v1::{TokenLimits, TokenPreset};
use scylla_testkit::{text, TestApp};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::app;

/// Posts a JSON body to a method of the gateway with an optional bearer token, and returns the
/// HTTP status and the JSON answer.
async fn post(app: &TestApp, method: &str, token: Option<&str>, body: &str) -> (u16, Value) {
    let url = format!("http://{}/api/{method}", app.addr);
    let mut request = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/json")
        .body(body.to_owned());
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request.send().await.unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap_or(Value::Null))
}

/// The method that lists the projects of the caller.
const LIST_PROJECTS: &str = "scylla.projects.v1.ProjectService/ListProjects";

/// The method that creates a project.
const CREATE_PROJECT: &str = "scylla.projects.v1.ProjectService/CreateProject";

/// A method takes and gives the proto3 JSON of its messages.
#[sqlx::test(migrations = false)]
async fn the_gateway_calls_a_method_in_json(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let token = app.token(user).await;
    let body = json!({ "workspaceId": { "value": workspace.to_string() }, "name": "alpha" });

    // when
    let (created, project) = post(&app, CREATE_PROJECT, Some(&token), &body.to_string()).await;
    let (listed, projects) = post(&app, LIST_PROJECTS, Some(&token), "").await;

    // then
    assert_eq!((created, listed), (200, 200), "{project} {projects}");
    assert_eq!(project["project"]["name"], "alpha");
    assert_eq!(projects["projects"][0]["name"], "alpha");
}

/// An error has the HTTP status of its gRPC code, its code name, and its fields.
#[sqlx::test(migrations = false)]
async fn the_gateway_answers_the_errors_in_json(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let token = Some(app.token(user).await);
    let token = token.as_deref();
    let no_name = json!({ "workspaceId": { "value": workspace.to_string() }, "name": " " });

    // when
    let (anonymous, refused) = post(&app, LIST_PROJECTS, None, "").await;
    let (garbage, not_json) = post(&app, CREATE_PROJECT, token, "{ nope").await;
    let (empty, invalid) = post(&app, CREATE_PROJECT, token, &no_name.to_string()).await;
    let (unknown, _) = post(&app, "scylla.projects.v1.ProjectService/Nope", token, "").await;
    let tail = "scylla.pipelines.v1.JobService/TailJobLogs";
    let (stream, _) = post(&app, tail, token, "").await;

    // then
    assert_eq!(
        (anonymous, refused["code"].clone()),
        (401, json!("UNAUTHENTICATED"))
    );
    assert_eq!(
        (garbage, not_json["code"].clone()),
        (400, json!("INVALID_ARGUMENT"))
    );
    assert_eq!(empty, 400);
    assert_eq!(invalid["fieldViolations"][0]["field"], "name");
    assert_eq!((unknown, stream), (404, 400));
}

/// The secret of an App token or of a personal token works directly, and a revoked API token
/// stops at once, also after a first call.
#[sqlx::test(migrations = false)]
async fn an_api_token_calls_the_gateway_directly(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    app.grant(&[Workspace::ADMIN.of(workspace, key.actor())])
        .await;
    let full = Some(TokenLimits {
        preset: TokenPreset::Full as i32,
        ..TokenLimits::default()
    });
    let created = app.create_limited_app_token(owner, &key.app, "ci", 0, full);
    let created = created.await.unwrap();
    let id = text(&created.token.unwrap().id).to_owned();
    let personal = app.create_personal_token(owner, "me", None).await.unwrap();

    // when
    let (first, _) = post(&app, LIST_PROJECTS, Some(&created.secret), "").await;
    let (second, _) = post(&app, LIST_PROJECTS, Some(&created.secret), "").await;
    app.revoke_app_token(owner, &key.app, &id).await.unwrap();
    let (revoked, _) = post(&app, LIST_PROJECTS, Some(&created.secret), "").await;
    let (by_person, _) = post(&app, LIST_PROJECTS, Some(&personal.secret), "").await;
    let (unknown, _) = post(&app, LIST_PROJECTS, Some("scya_unknown"), "").await;

    // then
    assert_eq!((first, second, revoked), (200, 200, 401));
    assert_eq!((by_person, unknown), (200, 401));
}
