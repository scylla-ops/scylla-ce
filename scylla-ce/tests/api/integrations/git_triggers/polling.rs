//! The test of a run that a poll starts: a repository in polling mode gets no webhook, and a new
//! head that a poll finds starts the run of its commit.

use scylla_proto::pipelines::v1::SourceEvent;
use serde_json::json;
use sqlx::PgPool;

use super::{app_with_fake, jobs_of, linked, pipeline_with, OTHER_SHA, SHA};

/// A poll of a link in polling mode finds a new head of `main`, and the push trigger of `main`
/// starts a run on the new commit, with its message.
#[sqlx::test(migrations = false)]
async fn a_poll_in_polling_mode_starts_the_run_of_a_new_head(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let rules = [("push", json!({ "branches": ["main"] }))];
    let (pipeline, _) = pipeline_with(&app, &linked, &rules).await;
    for (name, sha) in [
        ("branches", None),
        ("pulls", None),
        ("refs/heads/main", Some(SHA)),
    ] {
        app.insert_poll_state(&linked.project, name, sha).await;
    }
    fake.answer_github_heads("octo/app", &[("main", OTHER_SHA)], json!([]), "\"e-1\"");
    let reason = Some("WEBHOOKS_NOT_ARRIVING");
    app.set_link_mode(&linked.project, "polling", reason).await;

    // when
    let since = app.poll_now(&linked.project).await;
    app.wait_for_poll(&linked.project, &since).await;
    let key = format!("poll:octo/app:refs/heads/main:{OTHER_SHA}");
    let done = app.wait_for_delivery(&key).await;

    // then
    assert_eq!(done.error, "");
    let jobs = jobs_of(&app, &linked, &pipeline).await;
    assert_eq!(jobs.len(), 1);
    let source = jobs[0].source.as_ref().unwrap();
    assert_eq!(source.event(), SourceEvent::Push);
    assert_eq!(
        (source.sha.as_str(), source.branch.as_str()),
        (OTHER_SHA, "main")
    );
    assert_eq!(source.message_title, format!("Commit {OTHER_SHA}"));
}
