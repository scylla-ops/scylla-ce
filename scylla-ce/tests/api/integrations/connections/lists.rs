//! The tests of the reads of the connections: the providers, the list and the read of the
//! connections, and the repositories that a connection sees.

use scylla_proto::integrations::v1::{
    ListConnectionRepositoriesResponse, VcsConnection, VcsProvider, VcsProviderInfo,
};
use scylla_testkit::LinkRow;
use serde_json::{json, Value};
use sqlx::PgPool;
use tonic::Code;

use super::{app_with_fake, assert_reason, permissions};
use crate::{app, assert_code, UNKNOWN_ID};

/// A server with the App and a key gives GitHub and GitLab. A server with no App cannot connect
/// GitHub.
#[sqlx::test(migrations = false)]
async fn get_vcs_providers_tells_what_a_workspace_can_connect(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool.clone()).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();

    // when
    let providers = app.get_vcs_providers(owner, &workspace).await.unwrap();

    // then
    let kinds: Vec<_> = providers.iter().map(|info| info.provider).collect();
    assert_eq!(
        kinds,
        [VcsProvider::Github as i32, VcsProvider::Gitlab as i32]
    );
    assert!(providers.iter().all(|info| info.available));
    assert_eq!(providers[0].web_url, fake.url());
    let bare = providers_without_github(pool).await;
    assert!(!bare[0].available);
    assert_eq!(bare[0].unavailable_reason, "APP_MISSING");
    assert!(bare[1].available);
}

/// Returns the providers of a workspace on a server with no GitHub App.
async fn providers_without_github(pool: PgPool) -> Vec<VcsProviderInfo> {
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    app.get_vcs_providers(owner, &workspace).await.unwrap()
}

/// The list gives the connections of the workspace oldest first, by pages, with the count of
/// the projects that link a repository of each one.
#[sqlx::test(migrations = false)]
async fn list_vcs_connections_gives_the_oldest_first(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace_id = app.workspace_of(owner).await;
    let workspace = workspace_id.to_string();
    let first = app
        .insert_github_connection(workspace_id, owner, 7, "acme")
        .await;
    let second = app
        .insert_github_connection(workspace_id, owner, 8, "beta")
        .await;
    let third = app
        .insert_github_connection(workspace_id, owner, 9, "gamma")
        .await;
    let project = app.create_project(owner, "API").await.unwrap();
    let link = LinkRow {
        project: &project.id.unwrap().value,
        connection: &first,
        external_id: "1",
        full_name: "acme/api",
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;

    // when
    let page = app
        .list_vcs_connections(owner, &workspace, 2, "")
        .await
        .unwrap();
    let token = page.page.clone().unwrap().next_token;
    let last = app
        .list_vcs_connections(owner, &workspace, 2, &token)
        .await
        .unwrap();
    let one = app.get_vcs_connection(owner, &second).await.unwrap();

    // then
    let ids = |list: &[VcsConnection]| -> Vec<String> {
        list.iter().map(|c| c.id.clone().unwrap().value).collect()
    };
    assert_eq!(ids(&page.connections), [first.clone(), second.clone()]);
    assert_eq!(ids(&last.connections), [third]);
    assert_eq!(last.page.unwrap().next_token, "");
    assert_eq!(page.connections[0].project_count, 1);
    assert_eq!(one.account, "beta");
    assert_eq!(one.project_count, 0);
}

/// An unknown connection and a connection of another workspace get the answer of a refusal.
#[sqlx::test(migrations = false)]
async fn get_vcs_connection_of_an_unknown_or_foreign_connection_is_denied(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let connection = app
        .insert_github_connection(workspace, owner, 7, "acme")
        .await;

    // when
    let unknown = app.get_vcs_connection(owner, UNKNOWN_ID).await;
    let foreign = app.get_vcs_connection(stranger, &connection).await;

    // then
    assert_code(unknown, Code::PermissionDenied);
    assert_code(foreign, Code::PermissionDenied);
}

/// The repositories of a GitHub installation come with an installation token, filtered by the
/// search, by pages, with the project that links each one.
#[sqlx::test(migrations = false)]
async fn list_connection_repositories_of_github(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let connection = app
        .insert_github_connection(workspace, owner, 7, "acme")
        .await;
    let repositories = [
        (1, "acme/api", false),
        (2, "acme/api-docs", true),
        (3, "acme/web", false),
    ];
    fake.answer_github_install(7, "acme", permissions(), &repositories);
    let project = app
        .create_project(owner, "API")
        .await
        .unwrap()
        .id
        .unwrap()
        .value;
    let link = LinkRow {
        project: &project,
        connection: &connection,
        external_id: "1",
        full_name: "acme/api",
        default_branch: "main",
    };
    app.insert_repository_link(&link).await;

    // when
    let list = |token: &'static str| {
        app.list_connection_repositories(owner, &project, &connection, "API", 1, token)
    };
    let first = list("").await.unwrap();
    let second = list("1").await.unwrap();

    // then
    let names = |page: &ListConnectionRepositoriesResponse| {
        page.repositories
            .iter()
            .map(|r| r.full_name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&first), ["acme/api"]);
    assert_eq!(names(&second), ["acme/api-docs"]);
    assert_eq!(first.page.unwrap().next_token, "1");
    assert_eq!(second.page.unwrap().next_token, "");
    assert_eq!(
        first.repositories[0]
            .linked_project_id
            .as_ref()
            .unwrap()
            .value,
        project
    );
    assert!(second.repositories[0].linked_project_id.is_none());
    assert!(second.repositories[0].private);
    let token = &fake.requests_to("POST", "/app/installations/7/access_tokens")[0];
    assert_eq!(token.body, json!({ "permissions": { "metadata": "read" } }));
    let read = &fake.requests_to("GET", "/installation/repositories")[0];
    assert_eq!(
        read.header("authorization"),
        Some("Bearer ghs_installation")
    );
}

/// The projects of a GitLab connection come with its token and the search, with the role of
/// the service account.
#[sqlx::test(migrations = false)]
async fn list_connection_repositories_of_gitlab(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let token = "glpat-list";
    let connection = app
        .insert_gitlab_connection(workspace, owner, "scylla-bot", token)
        .await;
    let projects = [(10, "bank/payments", 30), (11, "bank/payments-api", 40)];
    fake.answer_gitlab_token((1, "scylla-bot", true), &["api"], Value::Null, &projects);
    let project = app
        .create_project(owner, "Payments")
        .await
        .unwrap()
        .id
        .unwrap()
        .value;

    // when
    let page = app
        .list_connection_repositories(owner, &project, &connection, "payments", 0, "")
        .await
        .unwrap();

    // then
    let roles: Vec<_> = page.repositories.iter().map(|r| r.role.as_str()).collect();
    assert_eq!(roles, ["developer", "maintainer"]);
    let read = &fake.requests_to("GET", "/api/v4/projects")[0];
    assert_eq!(read.header("private-token"), Some(token));
    assert!(read.query.contains("search=payments"), "{}", read.query);
}

/// A connection of another workspace is denied, and a revoked token gives `PROVIDER_REFUSED`.
#[sqlx::test(migrations = false)]
async fn list_connection_repositories_refuses_a_foreign_or_refused_connection(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let stranger = app.user().await;
    let theirs = app.workspace_of(stranger).await;
    let foreign = app
        .insert_github_connection(theirs, stranger, 7, "acme")
        .await;
    let workspace = app.workspace_of(owner).await;
    let mine = app
        .insert_gitlab_connection(workspace, owner, "bot", "glpat-revoked")
        .await;
    let refused = json!({ "message": "401 Unauthorized" });
    fake.respond("GET", "/api/v4/projects", 401, refused, &[]);
    let project = app
        .create_project(owner, "API")
        .await
        .unwrap()
        .id
        .unwrap()
        .value;

    // when
    let denied = app
        .list_connection_repositories(owner, &project, &foreign, "", 0, "")
        .await;
    let revoked = app
        .list_connection_repositories(owner, &project, &mine, "", 0, "")
        .await;

    // then
    assert_code(denied, Code::PermissionDenied);
    assert_reason(revoked, "PROVIDER_REFUSED");
}
