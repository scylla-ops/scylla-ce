//! The tests of the link of a GitHub repository: the check that the installation sees the
//! repository, and the refusals of a second link and of a repository that it does not see.

use scylla_proto::integrations::v1::{
    ConnectionState, DeliveryMode, PipelineSource, ReportMode, VcsProvider,
};
use serde_json::json;
use sqlx::PgPool;

use super::{app_with_fake, assert_reason, project};
use crate::assert_invalid_field;

/// The permissions of an installation of the test App.
fn permissions() -> serde_json::Value {
    json!({ "contents": "read", "metadata": "read" })
}

/// Story 8.1, step 4: an editor links a repository that the installation sees. The link has the
/// default branch, the check run mode and the webhook mode, and the readers of the project read
/// it.
#[sqlx::test(migrations = false)]
async fn link_repository_links_a_github_repository(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let project = project(&app, "github").await;
    let repositories = [(5, "octo/web", false), (7, "octo/api", true)];
    fake.answer_github_install(42, "octo", permissions(), &repositories);

    // when
    let linked = app
        .link_repository(project.owner, &project.id, &project.connection, "7")
        .await
        .unwrap();
    let read = app.get_repository_link(project.owner, &project.id).await;

    // then
    assert_eq!(linked.provider, VcsProvider::Github as i32);
    assert_eq!(linked.account, "octo");
    assert_eq!(
        (linked.external_id.as_str(), linked.full_name.as_str()),
        ("7", "octo/api")
    );
    assert_eq!(linked.web_url, "https://github.com/octo/api");
    assert_eq!(linked.default_branch, "main");
    assert!(linked.private && linked.reports_enabled);
    assert_eq!(linked.report_mode, ReportMode::CheckRun as i32);
    assert_eq!(linked.delivery_mode, DeliveryMode::Webhook as i32);
    assert_eq!(linked.pipeline_source, PipelineSource::Editor as i32);
    assert_eq!(linked.state, ConnectionState::Active as i32);
    assert!(linked.state_reason.is_empty());
    assert_eq!(linked.poll_interval_seconds, 120);
    assert_eq!(read.unwrap(), Some(linked));
    let tokens = fake.requests_to("POST", "/app/installations/42/access_tokens");
    assert_eq!(
        tokens[0].body,
        json!({ "permissions": { "metadata": "read" } })
    );
}

/// A repository that the installation does not see gives `REPOSITORY_NOT_VISIBLE`, a second link
/// gives `REPOSITORY_ALREADY_LINKED`, and an id that is not a number is a bad request.
#[sqlx::test(migrations = false)]
async fn link_repository_refuses_a_second_link_and_an_unseen_repository(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let project = project(&app, "github").await;
    fake.answer_github_install(42, "octo", permissions(), &[(7, "octo/api", false)]);
    let link =
        |id: &'static str| app.link_repository(project.owner, &project.id, &project.connection, id);

    // when
    let unseen = link("99").await;
    let bad = link("octo/api").await.unwrap_err();
    let first = link("7").await;
    let calls = fake.requests().len();
    let second = link("7").await;

    // then
    assert_reason(unseen, "REPOSITORY_NOT_VISIBLE");
    assert_invalid_field(&bad, "external_id");
    assert!(first.is_ok());
    assert_reason(second, "REPOSITORY_ALREADY_LINKED");
    assert_eq!(fake.requests().len(), calls);
}

/// A server with no GitHub App cannot link a GitHub repository: `APP_MISSING`.
#[sqlx::test(migrations = false)]
async fn link_repository_needs_the_github_app(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let project = project(&app, "github").await;

    // when
    let refused = app
        .link_repository(project.owner, &project.id, &project.connection, "7")
        .await;

    // then
    assert_reason(refused, "APP_MISSING");
    let read = app.get_repository_link(project.owner, &project.id).await;
    assert_eq!(read.unwrap(), None);
}
