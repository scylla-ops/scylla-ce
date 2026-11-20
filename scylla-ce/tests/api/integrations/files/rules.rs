//! The tests of the triggers of the pipelines of files: the read only triggers of the `on` table
//! of a file, a schedule of a file pipeline, and the triggers of an archived pipeline that stop.

use scylla_proto::integrations::v1::PipelineSource;
use scylla_proto::pipelines::v1::trigger::Kind;
use scylla_testkit::{text, FakeProvider, TestApp};
use sqlx::PgPool;

use super::{
    app_with, by_key, linked, pipelines, BUILD, BUILD_PATH, DEPLOY, DEPLOY_PATH, MAIN_SHA,
};

/// Sends a webhook request with no signature, and returns the status.
async fn post(app: &TestApp, trigger: &str) -> u16 {
    let response = reqwest::Client::new()
        .post(format!("http://{}/webhooks/{trigger}", app.addr))
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    response.status().as_u16()
}

/// A file pipeline lists the triggers of the `on` table of its file first, read only, then its
/// own schedules. A file with no `on` table gives no trigger.
#[sqlx::test(migrations = false)]
async fn list_pipeline_triggers_gives_the_triggers_of_the_file(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    let files = [(BUILD_PATH, BUILD), (DEPLOY_PATH, DEPLOY)];
    fake.github_files("heads/main", MAIN_SHA, &files);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let all = pipelines(&app, owner, project).await;
    let build = text(&by_key(&all, "build").id).to_owned();
    let deploy = text(&by_key(&all, "deploy").id).to_owned();
    let nightly = app.create_trigger(owner, &build, "nightly", Some("0 3 * * *"), Vec::new());
    nightly.await.unwrap();

    // when
    let listed = app.list_pipeline_triggers(owner, &build).await.unwrap();
    let none = app.list_pipeline_triggers(owner, &deploy).await.unwrap();

    // then
    let triggers = listed.triggers;
    assert_eq!(triggers.len(), 2);
    let from_file = &triggers[0];
    assert!(from_file.from_file && from_file.enabled);
    assert_eq!(from_file.name, "on.push");
    let Some(Kind::Push(push)) = &from_file.kind else {
        panic!("a push trigger: {from_file:?}");
    };
    assert_eq!(push.branches, vec!["main", "release/*"]);
    assert!(push.skip_ci);
    assert!(!triggers[1].from_file && triggers[1].name == "nightly");
    assert!(none.triggers.is_empty());
}

/// The webhook of an archived pipeline stops: the route answers 404 before it reads the
/// signature. It works again when the pipeline comes back.
#[sqlx::test(migrations = false)]
async fn the_webhook_of_an_archived_pipeline_stops(pool: PgPool) {
    // given
    let fake = FakeProvider::start().await;
    fake.github_files("heads/main", MAIN_SHA, &[(BUILD_PATH, BUILD)]);
    let app = app_with(pool, &fake).await;
    let linked = linked(&app, "github").await;
    let (owner, project) = (linked.owner, linked.project.as_str());
    let hook = app.create_trigger(owner, &linked.editor, "hook", None, Vec::new());
    let hook = hook.await.unwrap().trigger.unwrap();
    let hook = text(&hook.id).to_owned();
    let active = post(&app, &hook).await;

    // when
    app.set_pipeline_source(owner, project, PipelineSource::File)
        .await
        .unwrap();
    let archived = post(&app, &hook).await;
    app.set_pipeline_source(owner, project, PipelineSource::Editor)
        .await
        .unwrap();
    let restored = post(&app, &hook).await;

    // then
    assert_eq!((active, archived, restored), (401, 404, 401));
}
