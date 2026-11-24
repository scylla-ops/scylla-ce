//! The tests of the CI identities of the Apps: a CI job gets a token of its App with the ID
//! token of its job, inside the rules of the CI identity, with no secret.

use std::time::{SystemTime, UNIX_EPOCH};

use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::apps::v1::{AddCiIdentityRequest, CiProvider};
use scylla_proto::auth::v1::{TokenLimits, TokenPreset};
use scylla_proto::projects::v1::ListProjectsRequest;
use scylla_testkit::{id, text, with_token, TestApp};
use serde_json::json;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::identity::idp::Idp;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Returns `Ok` if a token can call a route that each actor can call.
async fn try_token(app: &TestApp, token: &str) -> Result<(), Status> {
    let list = ListProjectsRequest {
        workspace_id: None,
        page: None,
    };
    let mut client = app.projects.clone();
    client.list_projects(with_token(token, list)).await?;
    Ok(())
}

/// Makes the request of a CI identity of the fake provider.
fn rule(
    idp: &Idp,
    app: &str,
    provider: CiProvider,
    repository: &str,
    git_ref: &str,
    environment: &str,
) -> AddCiIdentityRequest {
    AddCiIdentityRequest {
        app_id: Some(id(app)),
        provider: provider as i32,
        issuer: idp.url().to_owned(),
        repository: repository.to_owned(),
        git_ref: git_ref.to_owned(),
        environment: environment.to_owned(),
        limits: Some(TokenLimits {
            preset: TokenPreset::Full as i32,
            ..TokenLimits::default()
        }),
    }
}

/// Signs the ID token of a job for the server, with more claims.
fn job(idp: &Idp, app: &TestApp, claims: serde_json::Value) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut all = json!({
        "aud": format!("http://{}", app.addr),
        "sub": "repo:acme/web",
        "exp": now + 300,
    });
    for (name, value) in claims.as_object().unwrap() {
        all[name] = value.clone();
    }
    idp.sign_claims(all)
}

/// A GitHub job of the repository and the ref of the rule gets a token of the App, and the
/// list shows its last use.
#[sqlx::test(migrations = false)]
async fn a_ci_job_gets_a_token_of_its_app(pool: PgPool) {
    // given
    let idp = Idp::start().await;
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    let github = CiProvider::Github;
    let request = rule(&idp, &key.app, github, "acme/web", "refs/heads/main", "");
    app.add_ci_identity(owner, request).await.unwrap();
    let claims = json!({ "repository": "acme/web", "ref": "refs/heads/main" });

    // when
    let token = app
        .exchange_ci_token(&key.app, &job(&idp, &app, claims))
        .await;

    // then
    assert!(try_token(&app, &token.unwrap()).await.is_ok());
    let listed = app.list_ci_identities(owner, &key.app).await.unwrap();
    assert!(listed[0].last_used_at.is_some());
    assert_eq!(listed[0].issuer, idp.url());
}

/// A job of another repository or ref, a token for another audience, another App, a removed
/// rule and a disabled App get no token.
#[sqlx::test(migrations = false)]
async fn a_job_out_of_the_rules_gets_no_token(pool: PgPool) {
    // given
    let idp = Idp::start().await;
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    let (other, _) = app.app_in(owner, workspace, false).await;
    let github = CiProvider::Github;
    let request = rule(&idp, &key.app, github, "acme/web", "refs/heads/main", "");
    let identity = app.add_ci_identity(owner, request).await.unwrap();
    let good = job(
        &idp,
        &app,
        json!({ "repository": "acme/web", "ref": "refs/heads/main" }),
    );
    let repository = job(
        &idp,
        &app,
        json!({ "repository": "acme/api", "ref": "refs/heads/main" }),
    );
    let git_ref = job(
        &idp,
        &app,
        json!({ "repository": "acme/web", "ref": "refs/heads/dev" }),
    );
    let audience = job(
        &idp,
        &app,
        json!({ "repository": "acme/web", "aud": "other" }),
    );

    // when
    let wrong_repository = app.exchange_ci_token(&key.app, &repository).await;
    let wrong_ref = app.exchange_ci_token(&key.app, &git_ref).await;
    let wrong_audience = app.exchange_ci_token(&key.app, &audience).await;
    let wrong_app = app.exchange_ci_token(&other.app, &good).await;
    app.disable_app(owner, &key.app).await.unwrap();
    let disabled = app.exchange_ci_token(&key.app, &good).await;
    app.enable_app(owner, &key.app).await.unwrap();
    let identity = text(&identity.id).to_owned();
    app.remove_ci_identity(owner, &key.app, &identity)
        .await
        .unwrap();
    let removed = app.exchange_ci_token(&key.app, &good).await;

    // then
    assert_code(wrong_repository, Code::Unauthenticated);
    assert_code(wrong_ref, Code::Unauthenticated);
    assert_code(wrong_audience, Code::Unauthenticated);
    assert_code(wrong_app, Code::Unauthenticated);
    let disabled = disabled.unwrap_err();
    let reason = disabled.get_details_error_info().map(|info| info.reason);
    assert_eq!(reason.as_deref(), Some("APP_DISABLED"));
    assert_code(removed, Code::Unauthenticated);
}

/// A GitLab rule reads the project path, and a rule with an environment needs it.
#[sqlx::test(migrations = false)]
async fn a_gitlab_rule_reads_its_claims(pool: PgPool) {
    // given
    let idp = Idp::start().await;
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    let gitlab = CiProvider::Gitlab;
    let request = rule(&idp, &key.app, gitlab, "group/project", "", "prod");
    app.add_ci_identity(owner, request).await.unwrap();
    let prod = json!({ "project_path": "group/project", "ref": "main", "environment": "prod" });
    let none = json!({ "project_path": "group/project", "ref": "main" });

    // when
    let with_environment = app
        .exchange_ci_token(&key.app, &job(&idp, &app, prod))
        .await;
    let without = app
        .exchange_ci_token(&key.app, &job(&idp, &app, none))
        .await;

    // then
    assert!(with_environment.is_ok());
    assert_code(without, Code::Unauthenticated);
}

/// The admins add and remove the CI identities, the readers list them, and a bad rule is
/// refused.
#[sqlx::test(migrations = false)]
async fn the_ci_identity_routes_are_checked(pool: PgPool) {
    // given
    let idp = Idp::start().await;
    let app = app(pool).await;
    let (owner, member) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let (key, _) = app.app_in(owner, workspace, false).await;
    let github = CiProvider::Github;
    let request = || rule(&idp, &key.app, github, "acme/web", "", "");
    app.add_ci_identity(owner, request()).await.unwrap();

    // when
    let by_member = app.add_ci_identity(member, request()).await;
    let listed = app.list_ci_identities(member, &key.app).await;
    let no_provider = app
        .add_ci_identity(
            owner,
            AddCiIdentityRequest {
                provider: 0,
                ..request()
            },
        )
        .await;
    let no_owner = AddCiIdentityRequest {
        repository: "web".to_owned(),
        ..request()
    };
    let no_owner = app.add_ci_identity(owner, no_owner).await;
    let unknown = app.remove_ci_identity(owner, &key.app, UNKNOWN_ID).await;

    // then
    assert_code(by_member, Code::PermissionDenied);
    assert_eq!(listed.unwrap().len(), 1);
    assert_invalid_field(&no_provider.unwrap_err(), "provider");
    assert_invalid_field(&no_owner.unwrap_err(), "repository");
    assert_code(unknown, Code::NotFound);
}
