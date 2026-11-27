//! The tests of the text filters of the job list and of the trigger list of a workspace: the
//! number, the branch and the pipeline of a job, the name, the pipeline and the project of a
//! trigger, and a text that is too long.

use scylla_proto::pipelines::v1::{
    JobFilter, ListWorkspaceJobsRequest, ListWorkspaceTriggersRequest,
};
use scylla_testkit::{id, text, TestApp};
use sqlx::PgPool;

use super::attention::ended;
use super::job_stats::{place, Place};
use crate::{app, assert_invalid_field};

/// Returns the request of the jobs of the workspace of a place that match a text.
fn jobs_request(place: &Place, query: &str) -> ListWorkspaceJobsRequest {
    ListWorkspaceJobsRequest {
        workspace_id: Some(id(place.workspace.to_string())),
        filter: Some(JobFilter {
            query: query.to_owned(),
            ..JobFilter::default()
        }),
        ..ListWorkspaceJobsRequest::default()
    }
}

/// Returns the sorted ids of the jobs of the workspace of a place that match a text.
async fn jobs(app: &TestApp, place: &Place, query: &str) -> Vec<String> {
    let request = jobs_request(place, query);
    let response = app.list_workspace_jobs_of(place.owner, request).await;
    let mut ids: Vec<_> = response
        .unwrap()
        .jobs
        .iter()
        .map(|job| text(&job.id).to_owned())
        .collect();
    ids.sort();
    ids
}

/// Returns the request of the triggers of the workspace of a place that match a text.
fn triggers_request(place: &Place, query: &str) -> ListWorkspaceTriggersRequest {
    ListWorkspaceTriggersRequest {
        workspace_id: Some(id(place.workspace.to_string())),
        query: query.to_owned(),
        ..ListWorkspaceTriggersRequest::default()
    }
}

/// Returns the sorted names of the triggers of the workspace of a place that match a text.
async fn triggers(app: &TestApp, place: &Place, query: &str) -> Vec<String> {
    let request = triggers_request(place, query);
    let response = app.list_workspace_triggers(place.owner, request).await;
    let summaries = response.unwrap().triggers;
    let mut names: Vec<_> = summaries
        .into_iter()
        .map(|summary| summary.trigger.unwrap().name)
        .collect();
    names.sort();
    names
}

/// Gives a new name to a pipeline of a place.
async fn rename(app: &TestApp, place: &Place, pipeline: usize, name: &str) {
    let pipeline = &place.pipelines[pipeline];
    let renamed = app.update_pipeline(place.owner, pipeline, 1, Some(name), None);
    renamed.await.unwrap();
}

/// The text finds a job by its number, the last 8 characters of its id, in any case.
#[sqlx::test(migrations = false)]
async fn list_workspace_jobs_finds_a_job_by_its_number(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let first = ended(&app, &place, 0, "failed", 20, None).await;
    ended(&app, &place, 2, "failed", 10, None).await;
    let number = first[first.len() - 8..].to_uppercase();

    // when
    let found = jobs(&app, &place, &number).await;

    // then
    assert_eq!(found, vec![first]);
}

/// The text finds a job by its branch or by the name of its pipeline, and an empty text gives
/// each job.
#[sqlx::test(migrations = false)]
async fn list_workspace_jobs_finds_a_job_by_its_branch_or_its_pipeline(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    rename(&app, &place, 2, "Deploy Web").await;
    let login = ended(&app, &place, 0, "failed", 30, Some("feature/Login")).await;
    let deploy = ended(&app, &place, 2, "completed", 20, None).await;
    let main = ended(&app, &place, 1, "failed", 10, Some("main")).await;

    // when
    let by_branch = jobs(&app, &place, "LOGIN").await;
    let by_pipeline = jobs(&app, &place, " deploy ").await;
    let none = jobs(&app, &place, "nothing").await;
    let each = jobs(&app, &place, "").await;

    // then
    assert_eq!(by_branch, vec![login.clone()]);
    assert_eq!(by_pipeline, vec![deploy.clone()]);
    assert!(none.is_empty());
    let mut all = vec![login, deploy, main];
    all.sort();
    assert_eq!(each, all);
}

/// The text finds a trigger by its name, by the name of its pipeline or by the name of its
/// project, in any case.
#[sqlx::test(migrations = false)]
async fn list_workspace_triggers_finds_a_trigger_by_its_name_pipeline_or_project(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    rename(&app, &place, 1, "Deploy Web").await;
    let cron = Some("0 3 * * *");
    for (pipeline, name) in [(0, "Cleanup cache"), (1, "hourly"), (2, "weekly")] {
        let created = app.create_trigger(
            place.owner,
            &place.pipelines[pipeline],
            name,
            cron,
            Vec::new(),
        );
        created.await.unwrap();
    }

    // when
    let by_name = triggers(&app, &place, "CACHE").await;
    let by_pipeline = triggers(&app, &place, "deploy").await;
    let by_project = triggers(&app, &place, "Beta").await;
    let each = triggers(&app, &place, "").await;

    // then
    assert_eq!(by_name, vec!["Cleanup cache"]);
    assert_eq!(by_pipeline, vec!["hourly"]);
    assert_eq!(by_project, vec!["weekly"]);
    assert_eq!(each, vec!["Cleanup cache", "hourly", "weekly"]);
}

/// A text of more than 100 characters is a bad request, in the job list and in the trigger list.
#[sqlx::test(migrations = false)]
async fn list_workspace_jobs_and_triggers_refuse_a_long_query(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let long = "x".repeat(101);

    // when
    let jobs = app.list_workspace_jobs_of(place.owner, jobs_request(&place, &long));
    let jobs = jobs.await.expect_err("jobs");
    let triggers = app.list_workspace_triggers(place.owner, triggers_request(&place, &long));
    let triggers = triggers.await.expect_err("triggers");

    // then
    assert_invalid_field(&jobs, "filter.query");
    assert_invalid_field(&triggers, "query");
}
