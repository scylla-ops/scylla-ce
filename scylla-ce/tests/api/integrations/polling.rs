//! The tests of the polling of the linked repositories: the first poll keeps the heads, a new
//! head goes to the path of a webhook, a link with no webhook in the window moves to the polling
//! mode, the ETag of a list, a GitLab webhook that GitLab disabled, and the live providers.

mod live;

use scylla_core::api::ServeOptions;
use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::DeliveryMode;
use scylla_testkit::{text, FakeProvider, LinkRow, TestApp};
use serde_json::{json, Value};
use sqlx::PgPool;

/// The old head of `main`.
const OLD: &str = "1111111111111111111111111111111111111111";

/// The new head of `main`.
const NEW: &str = "2222222222222222222222222222222222222222";

/// The head of a pull request.
const PULL: &str = "3333333333333333333333333333333333333333";

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

/// Makes a project linked in webhook mode to `octo/app` of the installation 42 on GitHub, or to
/// `group/app` with the webhook 9 on GitLab. The repository has the id 7.
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
            app.insert_gitlab_connection(workspace, owner, "scylla-bot", "glpat-poll")
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
        app.set_link_hook(&project, "9").await;
    }
    Linked {
        owner,
        project,
        connection,
    }
}

/// Keeps the poll state of a link that a poll read before, with `main` at its old head.
async fn polled_before(app: &TestApp, project: &str) {
    app.insert_poll_state(project, "branches", None).await;
    app.insert_poll_state(project, "pulls", None).await;
    app.insert_poll_state(project, "refs/heads/main", Some(OLD))
        .await;
}

/// Returns an open pull request #3 of `octo/app`, from a branch of the repository.
fn github_pull() -> Value {
    let side = |name: &str, sha: &str| json!({ "ref": name, "sha": sha, "repo": { "id": 7, "full_name": "octo/app" } });
    json!([{
        "number": 3, "title": "Add login", "html_url": "https://github.com/octo/app/pull/3",
        "draft": false, "head": side("login", PULL), "base": side("main", OLD),
        "user": { "login": "octocat", "avatar_url": null },
    }])
}

/// The first poll of a link keeps the heads of its branches and of its open pull requests, and
/// gives no event.
#[sqlx::test(migrations = false)]
async fn a_first_poll_keeps_the_heads(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    fake.answer_github_heads("octo/app", &[("main", OLD)], github_pull(), "\"e-1\"");

    // when
    let since = app.poll_now(&linked.project).await;
    app.wait_for_poll(&linked.project, &since).await;

    // then
    let heads = app.poll_heads(&linked.project).await;
    let expected = [("refs/heads/main", OLD), ("refs/pull/3/head", PULL)];
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(name, sha)| ((*name).to_owned(), (*sha).to_owned()))
        .collect();
    assert_eq!(heads, expected);
    assert!(app.deliveries().await.is_empty());
    let token = &fake.requests_to("POST", "/app/installations/42/access_tokens")[0];
    let scope = json!({ "contents": "read", "pull_requests": "read" });
    assert_eq!(token.body["permissions"], scope);
    assert_eq!(token.body["repository_ids"], json!([7]));
}

/// A poll in webhook mode finds a head that no webhook told: it goes to the path of a webhook
/// with the key `poll:<repo>:<ref>:<sha>`, and a link with no webhook in the window moves to the
/// polling mode with its connection. The next poll sends the ETag of the list, and a list that
/// did not change gives no event.
#[sqlx::test(migrations = false)]
async fn a_missed_head_moves_a_quiet_link_to_polling(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    polled_before(&app, &linked.project).await;
    fake.answer_github_heads("octo/app", &[("main", NEW)], json!([]), "\"e-1\"");
    fake.respond("GET", "/repos/octo/app/branches", 304, Value::Null, &[]);
    let key = format!("poll:octo/app:refs/heads/main:{NEW}");

    // when
    app.poll_now(&linked.project).await;
    let delivered = app.wait_for_delivery(&key).await;
    let mode = app.wait_for_link_mode(&linked.project, "polling").await;
    let since = app.poll_now(&linked.project).await;
    app.wait_for_poll(&linked.project, &since).await;

    // then
    assert_eq!(
        (delivered.event.as_str(), delivered.error.as_str()),
        ("push", "")
    );
    assert_eq!(mode.reason, "WEBHOOKS_NOT_ARRIVING");
    let link = app
        .get_repository_link(linked.owner, &linked.project)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(link.delivery_mode(), DeliveryMode::Polling);
    let connection = app.connection_mode(&linked.connection).await;
    assert_eq!(connection.delivery_mode, "polling");
    let reads = fake.requests_to("GET", "/repos/octo/app/branches");
    assert_eq!(reads[0].header("if-none-match"), None);
    assert_eq!(reads[1].header("if-none-match"), Some("\"e-1\""));
    assert_eq!(app.deliveries().await.len(), 1);
    let heads = app.poll_heads(&linked.project).await;
    assert_eq!(heads, vec![("refs/heads/main".to_owned(), NEW.to_owned())]);
}

/// GitLab disabled the webhook of a link in webhook mode: a poll moves the link to the polling
/// mode with `WEBHOOK_DISABLED`.
#[sqlx::test(migrations = false)]
async fn a_disabled_gitlab_webhook_moves_the_link_to_polling(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    fake.answer_gitlab_heads(7, &[("main", OLD)], json!([]), (9, "disabled"));

    // when
    app.poll_now(&linked.project).await;
    let mode = app.wait_for_link_mode(&linked.project, "polling").await;

    // then
    assert_eq!(
        (mode.state.as_str(), mode.reason.as_str()),
        ("active", "WEBHOOK_DISABLED")
    );
    let hook = &fake.requests_to("GET", "/api/v4/projects/7/hooks/9")[0];
    assert_eq!(hook.header("private-token"), Some("glpat-poll"));
}

/// A poll of a GitLab link finds a new merge request from a fork: it goes to the path of a
/// webhook as an opened pull request. The next poll asks only for the merge requests that
/// changed.
#[sqlx::test(migrations = false)]
async fn a_poll_finds_a_new_merge_request(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    app.set_link_mode(&linked.project, "polling", Some("ROLE_TOO_LOW"))
        .await;
    polled_before(&app, &linked.project).await;
    let request = json!([{
        "iid": 17, "title": "Add login", "web_url": "https://gitlab.com/group/app/-/merge_requests/17",
        "source_branch": "login", "target_branch": "main", "source_project_id": 8,
        "target_project_id": 7, "sha": PULL, "draft": false,
        "author": { "username": "eve", "avatar_url": null },
    }]);
    fake.answer_gitlab_heads(7, &[("main", OLD)], request, (9, "executable"));
    let fork = json!({ "id": 8, "path_with_namespace": "eve/app" });
    fake.respond("GET", "/api/v4/projects/8", 200, fork, &[]);
    let key = format!("poll:group/app:refs/merge-requests/17/head:{PULL}");

    // when
    app.poll_now(&linked.project).await;
    let delivered = app.wait_for_delivery(&key).await;
    let since = app.poll_now(&linked.project).await;
    app.wait_for_poll(&linked.project, &since).await;

    // then
    assert_eq!(
        (delivered.event.as_str(), delivered.action.as_str()),
        ("pull_request", "opened")
    );
    let reads = fake.requests_to("GET", "/api/v4/projects/7/merge_requests");
    assert!(!reads[0].query.contains("updated_after"));
    assert!(reads[1].query.contains("updated_after="));
    assert!(reads[1].query.contains("state=opened"));
    assert!(fake
        .requests_to("GET", "/api/v4/projects/7/hooks/9")
        .is_empty());
    let mode = app.link_mode(&linked.project).await;
    assert_eq!(mode.reason, "ROLE_TOO_LOW");
}
