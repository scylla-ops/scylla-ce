//! The tests of `ListPipelineSummaries`: the recent jobs, the health and the triggers of each
//! pipeline. The order, the pages and the access are in `pipeline_summary_lists.rs`.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::{
    JobStatus, ListPipelineSummariesRequest, PipelineSummary, PipelineSummaryOrder, TriggerKind,
};
use scylla_testkit::{exec_node, id, text, LinkRow, TestApp};
use sqlx::PgPool;

use crate::app;

/// Makes a request for the pipelines of a workspace.
pub(super) fn request(workspace: WorkspaceId) -> ListPipelineSummariesRequest {
    ListPipelineSummariesRequest {
        workspace_id: Some(id(workspace.to_string())),
        ..Default::default()
    }
}

/// Makes a pipeline with a name in a project, and returns its id.
pub(super) async fn named_pipeline(
    app: &TestApp,
    owner: ActorId,
    project: &str,
    name: &str,
) -> String {
    let pipeline = app
        .create_pipeline(owner, project, vec![exec_node("build", &[])])
        .await
        .unwrap();
    let pipeline = text(&pipeline.id).to_owned();
    app.update_pipeline(owner, &pipeline, 1, Some(name), None)
        .await
        .unwrap();
    pipeline
}

/// Runs a pipeline and gives its job a state. An ended job ran for `seconds` seconds.
pub(super) async fn job(
    app: &TestApp,
    owner: ActorId,
    pipeline: &str,
    status: &str,
    seconds: f64,
) -> String {
    let job = app.run_pipeline(owner, pipeline).await.unwrap();
    let job = text(&job.id).to_owned();
    if status != "pending" {
        sqlx::query(
            "UPDATE jobs SET status = $2, started_at = now() - make_interval(secs => $3),
                             finished_at = now()
             WHERE id = $1::uuid",
        )
        .bind(&job)
        .bind(status)
        .bind(seconds)
        .execute(&app.pool)
        .await
        .unwrap();
    }
    job
}

/// Returns the names of the pipelines of a list, in its order.
pub(super) fn names(pipelines: &[PipelineSummary]) -> Vec<&str> {
    pipelines
        .iter()
        .map(|summary| summary.name.as_str())
        .collect()
}

/// A summary gives the recent jobs newest first, the counts and the median of the jobs that
/// completed, the state of the last ended job, and a job that waits.
#[sqlx::test(migrations = false)]
async fn list_pipeline_summaries_gives_the_recent_jobs(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let pipeline = named_pipeline(&app, owner, text(&project.id), "build").await;
    let done = job(&app, owner, &pipeline, "completed", 10.0).await;
    job(&app, owner, &pipeline, "completed", 30.0).await;
    job(&app, owner, &pipeline, "cancelled", 1.0).await;
    let failed = job(&app, owner, &pipeline, "failed", 2.0).await;
    let waiting = job(&app, owner, &pipeline, "pending", 0.0).await;

    // when
    let all = app
        .list_pipeline_summaries(owner, request(workspace))
        .await
        .unwrap();
    let mut short = request(workspace);
    short.runs = 2;
    let short = app.list_pipeline_summaries(owner, short).await.unwrap();

    // then
    assert_eq!((all.total, all.green), (1, 0));
    let summary = &all.pipelines[0];
    assert_eq!(text(&summary.pipeline_id), pipeline);
    assert_eq!(summary.project_name, "alpha");
    assert_eq!(summary.recent.len(), 5);
    assert_eq!(text(&summary.recent[0].id), waiting);
    assert_eq!(text(&summary.recent[1].id), failed);
    assert_eq!(text(&summary.recent[4].id), done);
    assert_eq!(summary.recent[1].status, JobStatus::Failed as i32);
    assert!(summary.recent[0].finished_at.is_none());
    assert!(summary.recent[1].started_at.is_some());
    assert_eq!((summary.completed, summary.failed), (2, 1));
    let median = summary.median_duration.unwrap();
    assert!((median.seconds - 20).abs() <= 1, "{median:?}");
    assert_eq!(summary.last_ended_status, JobStatus::Failed as i32);
    assert!(summary.active);
    let summary = &short.pipelines[0];
    assert_eq!(summary.recent.len(), 2);
    assert_eq!((summary.completed, summary.failed), (0, 1));
    assert!(summary.median_duration.is_none());
}

/// With a linked repository, the health comes from the last ended job of the default branch.
#[sqlx::test(migrations = false)]
async fn list_pipeline_summaries_reads_the_default_branch(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let connection = app
        .insert_github_connection(workspace, owner, 7, "octo")
        .await;
    app.insert_repository_link(&LinkRow {
        project: &project,
        connection: &connection,
        external_id: "42",
        full_name: "octo/app",
        default_branch: "main",
    })
    .await;
    let pipeline = named_pipeline(&app, owner, &project, "build").await;
    let main = job(&app, owner, &pipeline, "completed", 5.0).await;
    app.insert_job_source(&main, "aaa", "main", "push").await;
    let feature = job(&app, owner, &pipeline, "failed", 5.0).await;
    app.insert_job_source(&feature, "bbb", "feature", "push")
        .await;
    job(&app, owner, &pipeline, "failed", 5.0).await;

    // when
    let list = app
        .list_pipeline_summaries(owner, request(workspace))
        .await
        .unwrap();

    // then
    let summary = &list.pipelines[0];
    assert_eq!(summary.last_ended_status, JobStatus::Completed as i32);
    assert!(!summary.active);
    assert_eq!(list.green, 1);
    assert_eq!(summary.recent[0].branch, "");
    assert_eq!(summary.recent[1].branch, "feature");
    assert_eq!(summary.recent[2].branch, "main");
}

/// A summary gives the kinds of the enabled triggers, of the on section of a pipeline file, and
/// the next fire of the schedules. A pipeline with no trigger and an archived pipeline have none.
#[sqlx::test(migrations = false)]
async fn list_pipeline_summaries_gives_the_triggers(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let project = text(&project.id).to_owned();
    let scheduled = named_pipeline(&app, owner, &project, "nightly").await;
    let manual = named_pipeline(&app, owner, &project, "manual").await;
    let cron = app
        .create_trigger(owner, &scheduled, "night", Some("0 3 * * *"), Vec::new())
        .await
        .unwrap()
        .trigger
        .unwrap();
    app.create_trigger(owner, &scheduled, "hook", None, Vec::new())
        .await
        .unwrap();
    let off = app
        .create_trigger(owner, &scheduled, "off", None, Vec::new())
        .await
        .unwrap()
        .trigger
        .unwrap();
    app.set_trigger_enabled(owner, text(&off.id), false)
        .await
        .unwrap();
    let files = named_pipeline(&app, owner, &project, "files").await;
    sqlx::query(
        "UPDATE pipelines SET source = 'file', file_key = 'ci',
                              file_path = '.scylla/pipelines/ci.toml',
                              file_rules = '{\"push\": {}, \"tag\": {}}'
         WHERE id = $1::uuid",
    )
    .bind(&files)
    .execute(&app.pool)
    .await
    .unwrap();
    let old = named_pipeline(&app, owner, &project, "old").await;
    app.create_trigger(owner, &old, "night", Some("0 4 * * *"), Vec::new())
        .await
        .unwrap();
    sqlx::query("UPDATE pipelines SET archived_at = now() WHERE id = $1::uuid")
        .bind(&old)
        .execute(&app.pool)
        .await
        .unwrap();

    // when
    let mut by_name = request(workspace);
    by_name.order = PipelineSummaryOrder::Name as i32;
    let list = app.list_pipeline_summaries(owner, by_name).await.unwrap();

    // then
    assert_eq!(
        names(&list.pipelines),
        ["files", "manual", "nightly", "old"]
    );
    let file_kinds = [TriggerKind::Push as i32, TriggerKind::Tag as i32];
    assert_eq!(list.pipelines[0].trigger_kinds, file_kinds);
    let manual_summary = &list.pipelines[1];
    assert_eq!(text(&manual_summary.pipeline_id), manual);
    assert!(manual_summary.trigger_kinds.is_empty());
    assert!(manual_summary.next_fire.is_none());
    let nightly = &list.pipelines[2];
    let kinds = [TriggerKind::Cron as i32, TriggerKind::Webhook as i32];
    assert_eq!(nightly.trigger_kinds, kinds);
    let fire = nightly.next_fire.as_ref().unwrap();
    assert_eq!(text(&fire.trigger_id), text(&cron.id));
    assert_eq!(fire.schedule, "0 3 * * *");
    assert_eq!(fire.at, cron.next_fire_at);
    assert!(list.pipelines[3].trigger_kinds.is_empty());
    assert!(list.pipelines[3].next_fire.is_none());
}
