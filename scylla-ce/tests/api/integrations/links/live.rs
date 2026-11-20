//! The tests of a link to a real project on GitLab.com. They are ignored: they read the account
//! of the `SCYLLA_LIVE_GITLAB_*` variables.

use scylla_testkit::text;
use sqlx::PgPool;

/// Returns a variable of the environment of the test.
fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name} for the live tests"))
}

/// The service account links the test project, then unlinks it. GitLab accepts the webhook of
/// a public URL, and refuses the URL of a local server: then the link polls.
#[ignore = "needs the GitLab service account and SCYLLA_LIVE_GITLAB_*"]
#[sqlx::test(migrations = false)]
async fn link_repository_on_a_live_gitlab_project(pool: PgPool) {
    // given
    let app = crate::app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "Live").await.unwrap();
    let project = text(&project.id).to_owned();
    let token = var("SCYLLA_LIVE_GITLAB_TOKEN");
    let connection = app
        .insert_gitlab_connection(workspace, owner, "scylla-bot", &token)
        .await;
    let external_id = var("SCYLLA_LIVE_GITLAB_PROJECT_ID");

    // when
    let linked = app
        .link_repository(owner, &project, &connection, &external_id)
        .await;
    let unlinked = app.unlink_repository(owner, &project).await;

    // then
    let linked = linked.unwrap();
    assert_eq!(linked.full_name, var("SCYLLA_LIVE_GITLAB_PROJECT"));
    assert!(unlinked.is_ok());
}
