//! The tests of the access to the connections, of their delete, of the logs, and of the live
//! GitHub App.

use scylla_core::infra::vcs::{
    Auth, GitHubAppSettings, Hidden, TokenScope, VcsClients, VcsOptions,
};
use scylla_testkit::{LinkRow, Logs, Role};
use serde_json::Value;
use sqlx::PgPool;
use tonic::Code;

use super::{app_with_fake, permissions, state_of};
use crate::assert_code;

/// A member and a viewer of a workspace read its connections. Only its admins and owners make,
/// change and delete them. A stranger reads nothing.
#[sqlx::test(migrations = false)]
async fn only_a_manager_changes_the_connections(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let (member, viewer, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace_id = app.workspace_of(owner).await;
    let workspace = workspace_id.to_string();
    for (actor, role) in [(member, "member"), (viewer, "viewer")] {
        let grant = Role::on("workspace", &workspace, role);
        app.grant_role(owner, grant, &actor.to_string())
            .await
            .unwrap();
    }
    let connection = app
        .insert_github_connection(workspace_id, owner, 7, "acme")
        .await;

    // when
    let viewer_list = app.list_vcs_connections(viewer, &workspace, 0, "").await;
    let member_get = app.get_vcs_connection(member, &connection).await;
    let member_start = app.start_github_install(member, &workspace).await;
    let member_connect = app.connect_gitlab(member, &workspace, "glpat-x").await;
    let member_replace = app
        .replace_gitlab_token(member, &connection, "glpat-x")
        .await;
    let member_delete = app.delete_vcs_connection(member, &connection).await;
    let stranger_list = app.list_vcs_connections(stranger, &workspace, 0, "").await;
    let stranger_providers = app.get_vcs_providers(stranger, &workspace).await;

    // then
    assert_eq!(viewer_list.unwrap().connections.len(), 1);
    assert!(member_get.is_ok());
    assert_code(member_start, Code::PermissionDenied);
    assert_code(member_connect, Code::PermissionDenied);
    assert_code(member_replace, Code::PermissionDenied);
    assert_code(member_delete, Code::PermissionDenied);
    assert_code(stranger_list, Code::PermissionDenied);
    assert_code(stranger_providers, Code::PermissionDenied);
}

/// A delete removes the connection with the links of its repositories. The GitLab webhooks go
/// first, with the token of the connection, and a webhook that GitLab does not delete stops
/// nothing.
#[sqlx::test(migrations = false)]
async fn delete_vcs_connection_deletes_the_links_and_the_gitlab_hooks(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let token = "glpat-delete";
    let connection = app
        .insert_gitlab_connection(workspace, owner, "bot", token)
        .await;
    for (name, external, hook) in [("Payments", "10", "99"), ("Ledger", "11", "98")] {
        let project = app
            .create_project(owner, name)
            .await
            .unwrap()
            .id
            .unwrap()
            .value;
        let link = LinkRow {
            project: &project,
            connection: &connection,
            external_id: external,
            full_name: name,
            default_branch: "main",
        };
        app.insert_repository_link(&link).await;
        app.set_link_hook(&project, hook).await;
    }
    fake.respond(
        "DELETE",
        "/api/v4/projects/10/hooks/99",
        204,
        Value::Null,
        &[],
    );
    fake.respond(
        "DELETE",
        "/api/v4/projects/11/hooks/98",
        500,
        Value::Null,
        &[],
    );

    // when
    app.delete_vcs_connection(owner, &connection).await.unwrap();
    let again = app.delete_vcs_connection(owner, &connection).await;

    // then
    let links: i64 = sqlx::query_scalar("SELECT count(*) FROM repository_links")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(links, 0);
    assert_code(again, Code::PermissionDenied);
    let deleted = fake.requests_to("DELETE", "/api/v4/projects/10/hooks/99");
    assert_eq!(deleted[0].header("private-token"), Some(token));
    assert_eq!(
        fake.requests_to("DELETE", "/api/v4/projects/11/hooks/98")
            .len(),
        1
    );
}

/// The install of the GitHub App and the connection of a GitLab token log their events, and no
/// log line and no answer has a token. A second capture stays open under the JSON capture: with
/// one capture only, `tracing` gives a log line that another test thread starts first the
/// interest of that thread, and the capture loses the line.
#[sqlx::test(migrations = false)]
async fn connections_log_no_token(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    fake.answer_github_install(7, "acme", permissions(), &[(1, "acme/api", false)]);
    fake.answer_gitlab_token((42, "scylla-bot", true), &["api"], Value::Null, &[]);
    let _under = Logs::pretty();
    let logs = Logs::json();

    // when
    let started = app.start_github_install(owner, &workspace).await.unwrap();
    let state = state_of(&started.install_url);
    let redirect = scylla_proto::integrations::v1::CompleteGitHubInstallRequest {
        state: state.clone(),
        installation_id: "7".to_owned(),
        code: "good-code".to_owned(),
        setup_action: "install".to_owned(),
    };
    let github = app.complete_github_install(owner, redirect).await.unwrap();
    let gitlab = app
        .connect_gitlab(owner, &workspace, "glpat-logged")
        .await
        .unwrap();

    // then
    let text = logs.text();
    let answers = format!("{github:?} {gitlab:?}");
    for secret in [
        "ghu_user",
        "ghs_installation",
        "glpat-logged",
        "good-code",
        &state,
    ] {
        assert!(!text.contains(secret), "{secret} is in the logs:\n{text}");
        assert!(!answers.contains(secret), "{secret} is in an answer");
    }
    assert_eq!(
        text.matches("\"event\":\"vcs.connected\"").count(),
        2,
        "{text}"
    );
}

/// Reads a variable of the live tests.
fn live(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} for the live tests"))
}

/// The dev GitHub App signs a JWT that GitHub accepts, and gets with it an installation token
/// of one hour. It needs the `SCYLLA_GITHUB_APP_*` settings of the dev App and
/// `SCYLLA_LIVE_GITHUB_INSTALLATION_ID`.
#[tokio::test]
#[ignore = "calls GitHub.com with the dev App"]
async fn live_github_app_gets_a_jwt_and_an_installation_token() {
    // given
    scylla_core::infra::tls::install();
    let key = match std::env::var("SCYLLA_GITHUB_APP_PRIVATE_KEY_FILE") {
        Ok(path) => std::fs::read_to_string(path).expect("the key file"),
        Err(_) => live("SCYLLA_GITHUB_APP_PRIVATE_KEY").replace("\\n", "\n"),
    };
    let settings = GitHubAppSettings {
        app_id: live("SCYLLA_GITHUB_APP_ID").parse().expect("an App id"),
        slug: live("SCYLLA_GITHUB_APP_SLUG"),
        client_id: live("SCYLLA_GITHUB_APP_CLIENT_ID"),
        client_secret: Hidden::new(live("SCYLLA_GITHUB_APP_CLIENT_SECRET")),
        private_key: Hidden::new(key),
        webhook_secret: Hidden::new(live("SCYLLA_GITHUB_APP_WEBHOOK_SECRET")),
    };
    let installation: u64 = live("SCYLLA_LIVE_GITHUB_INSTALLATION_ID").parse().unwrap();
    let options = VcsOptions {
        github_app: Some(settings),
        ..VcsOptions::default()
    };
    let clients = VcsClients::new(&options).expect("clients");
    let app = clients.github_app.as_ref().expect("the App");

    // when
    let scope = TokenScope::default().permission("metadata", "read");
    let token = app.installation_token(installation, &scope).await;

    // then
    let token = token.expect("GitHub accepts the JWT and gives a token");
    let read: Value = clients
        .github
        .get("/installation/repositories")
        .auth(Auth::Bearer(&token.token))
        .query("per_page", 1)
        .read()
        .await
        .expect("the token reads the repositories");
    assert!(read["total_count"].is_u64());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let life = token.expires_at.unix_timestamp() - i64::try_from(now).unwrap();
    assert!((3000..=3600).contains(&life), "{life}");
}
