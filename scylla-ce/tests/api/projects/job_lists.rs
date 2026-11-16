//! The tests of the job list of a workspace: the projects that the caller sees, the filters, the
//! summary view, and the origin of each job.

use std::time::{SystemTime, UNIX_EPOCH};

use prost_types::Timestamp;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::pipeline_service_client::PipelineServiceClient;
use scylla_proto::pipelines::v1::{
    JobFilter, JobOrigin, JobStatus, JobView, ListWorkspaceJobsRequest, RunPipelineRequest,
};
use scylla_testkit::{exec_node, id, text, with_token, Role, TestApp};
use sqlx::PgPool;

use crate::identity::accounts::PASSWORD;
use crate::{app, assert_invalid_field, UNKNOWN_ID};

/// A workspace with the project `alpha` of two pipelines and the project `beta` of one.
struct Place {
    /// The owner of the workspace.
    owner: ActorId,
    /// The workspace.
    workspace: WorkspaceId,
    /// The ids of `alpha` and `beta`.
    projects: [String; 2],
    /// The ids of the pipelines: two of `alpha`, then one of `beta`.
    pipelines: [String; 3],
}

/// Creates a user and its two projects. Each pipeline has one node with two retries.
async fn place(app: &TestApp) -> Place {
    let owner = app.user().await;
    let mut node = exec_node("build", &[]);
    node.retries = 2;
    let mut projects = Vec::new();
    let mut pipelines = Vec::new();
    for (name, count) in [("alpha", 2), ("beta", 1)] {
        let project = app.create_project(owner, name).await.unwrap();
        let project = text(&project.id).to_owned();
        for _ in 0..count {
            let pipeline = app.create_pipeline(owner, &project, vec![node.clone()]);
            pipelines.push(text(&pipeline.await.unwrap().id).to_owned());
        }
        projects.push(project);
    }
    Place {
        owner,
        workspace: app.workspace_of(owner).await,
        projects: projects.try_into().unwrap(),
        pipelines: pipelines.try_into().unwrap(),
    }
}

/// Starts a job of a pipeline as `actor`, and returns its id.
async fn run(app: &TestApp, actor: ActorId, pipeline: &str) -> String {
    text(&app.run_pipeline(actor, pipeline).await.unwrap().id).to_owned()
}

/// Returns the request of the jobs of a workspace with a filter.
fn request(workspace: WorkspaceId, filter: JobFilter) -> ListWorkspaceJobsRequest {
    ListWorkspaceJobsRequest {
        workspace_id: Some(id(workspace.to_string())),
        filter: Some(filter),
        ..ListWorkspaceJobsRequest::default()
    }
}

/// Returns the ids of the jobs of the first page of a list.
async fn list(app: &TestApp, actor: ActorId, request: ListWorkspaceJobsRequest) -> Vec<String> {
    let jobs = app
        .list_workspace_jobs_of(actor, request)
        .await
        .unwrap()
        .jobs;
    jobs.iter().map(|job| text(&job.id).to_owned()).collect()
}

/// Returns the time `days` days ago.
fn days_ago(days: u64) -> Option<Timestamp> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let seconds = i64::try_from(now.as_secs() - days * 86_400).unwrap();
    Some(Timestamp { seconds, nanos: 0 })
}

/// Sets the creation time of a job to `days` days ago.
async fn move_job(app: &TestApp, job: &str, days: i32) {
    sqlx::query!(
        "UPDATE jobs SET created_at = now() - make_interval(days => $2) WHERE id = $1",
        JobId::parse(job).unwrap().to_db(),
        days,
    )
    .execute(&app.pool)
    .await
    .unwrap();
}

/// A member with a role on one project sees only the jobs of this project, and a filter on
/// another project gives no job. A user out of the workspace gets an empty list, not a refusal.
#[sqlx::test(migrations = false)]
async fn a_member_sees_only_the_jobs_of_its_projects(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let alpha = run(&app, place.owner, &place.pipelines[0]).await;
    let beta = run(&app, place.owner, &place.pipelines[2]).await;
    let (viewer, stranger) = (app.user().await, app.user().await);
    app.join_workspace(place.owner, viewer).await;
    let role = Role::on("project", &place.projects[0], "viewer");
    app.grant_role(place.owner, role, &viewer.to_string())
        .await
        .unwrap();
    let all = request(place.workspace, JobFilter::default());
    let other = ListWorkspaceJobsRequest {
        project_ids: vec![id(place.projects[1].as_str())],
        ..all.clone()
    };

    // when
    let owned = list(&app, place.owner, all.clone()).await;
    let seen = list(&app, viewer, all.clone()).await;
    let hidden = list(&app, viewer, other).await;
    let none = list(&app, stranger, all).await;

    // then
    assert_eq!(owned, vec![beta, alpha.clone()]);
    assert_eq!(seen, vec![alpha]);
    assert!(hidden.is_empty(), "{hidden:?}");
    assert!(none.is_empty(), "{none:?}");
}

/// Each filter keeps the jobs that match it: the states, the origins, the times of creation,
/// the projects and the pipelines.
#[sqlx::test(migrations = false)]
async fn the_filters_narrow_the_list(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let owner = place.owner;
    let cancelled = run(&app, owner, &place.pipelines[0]).await;
    app.cancel_job(owner, &cancelled).await.unwrap();
    let second = run(&app, owner, &place.pipelines[1]).await;
    let other = run(&app, owner, &place.pipelines[2]).await;
    let rerun = app.rerun_job(owner, &cancelled).await.unwrap();
    let rerun = text(&rerun.id).to_owned();
    move_job(&app, &cancelled, 2).await;
    let with = |filter| request(place.workspace, filter);
    let all = with(JobFilter::default());

    // when
    let statuses = vec![JobStatus::Cancelled.into()];
    let by_status = with(JobFilter {
        statuses,
        ..JobFilter::default()
    });
    let by_status = list(&app, owner, by_status).await;
    let origins = vec![JobOrigin::Rerun.into()];
    let by_origin = with(JobFilter {
        origins,
        ..JobFilter::default()
    });
    let by_origin = list(&app, owner, by_origin).await;
    let older = with(JobFilter {
        created_before: days_ago(1),
        ..JobFilter::default()
    });
    let older = list(&app, owner, older).await;
    let newer = with(JobFilter {
        created_after: days_ago(1),
        ..JobFilter::default()
    });
    let newer = list(&app, owner, newer).await;
    let by_project = ListWorkspaceJobsRequest {
        project_ids: vec![id(place.projects[1].as_str())],
        ..all.clone()
    };
    let by_project = list(&app, owner, by_project).await;
    let by_pipeline = ListWorkspaceJobsRequest {
        pipeline_ids: vec![id(place.pipelines[1].as_str())],
        ..all
    };
    let by_pipeline = list(&app, owner, by_pipeline).await;

    // then
    assert_eq!(by_status, vec![cancelled.clone()]);
    assert_eq!(by_origin, vec![rerun.clone()]);
    assert_eq!(older, vec![cancelled]);
    assert_eq!(newer, vec![rerun, other.clone(), second.clone()]);
    assert_eq!(by_project, vec![other]);
    assert_eq!(by_pipeline, vec![second]);
}

/// An unknown state or origin in a filter, and more than 100 ids, give `INVALID_ARGUMENT`.
#[sqlx::test(migrations = false)]
async fn a_bad_filter_gives_invalid_argument(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let with = |filter| request(place.workspace, filter);
    let status = with(JobFilter {
        statuses: vec![JobStatus::Unspecified.into()],
        ..JobFilter::default()
    });
    let origin = with(JobFilter {
        origins: vec![99],
        ..JobFilter::default()
    });
    let many = ListWorkspaceJobsRequest {
        pipeline_ids: vec![id(UNKNOWN_ID); 101],
        ..with(JobFilter::default())
    };

    // when
    let status = app.list_workspace_jobs_of(place.owner, status).await;
    let origin = app.list_workspace_jobs_of(place.owner, origin).await;
    let many = app.list_workspace_jobs_of(place.owner, many).await;

    // then
    assert_invalid_field(&status.unwrap_err(), "filter.statuses");
    assert_invalid_field(&origin.unwrap_err(), "filter.origins");
    assert_invalid_field(&many.unwrap_err(), "pipeline_ids");
}

/// The summary view gives the jobs with their runs and no nodes. Each run gives its most
/// attempts: one and the retries of its node.
#[sqlx::test(migrations = false)]
async fn the_summary_view_has_no_nodes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let job = run(&app, place.owner, &place.pipelines[0]).await;
    let full = request(place.workspace, JobFilter::default());
    let summary = ListWorkspaceJobsRequest {
        view: JobView::Summary.into(),
        ..full.clone()
    };

    // when
    let full = app.list_workspace_jobs_of(place.owner, full).await;
    let summary = app.list_workspace_jobs_of(place.owner, summary).await;

    // then
    let full = &full.unwrap().jobs[0];
    assert_eq!(text(&full.id), job);
    assert_eq!(full.nodes.len(), 1);
    assert_eq!(full.runs[0].max_attempts, 3);
    let summary = &summary.unwrap().jobs[0];
    assert_eq!(text(&summary.id), job);
    assert!(summary.nodes.is_empty());
    assert_eq!(summary.runs[0].max_attempts, 3);
}

/// A run with the session of a person has the origin `MANUAL`, and a run with a token that has
/// no session has the origin `API`.
#[sqlx::test(migrations = false)]
async fn the_origin_tells_who_started_the_job(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "origin@example.com";
    let owner = app.user_with(email, Some(PASSWORD)).await;
    let project = app.create_project(owner, "alpha").await.unwrap();
    let nodes = vec![exec_node("build", &[])];
    let pipeline = app.create_pipeline(owner, text(&project.id), nodes);
    let pipeline = text(&pipeline.await.unwrap().id).to_owned();
    let session = app.sign_in(email, PASSWORD).await.unwrap();
    let session = session.token.unwrap().value;
    let request = RunPipelineRequest {
        pipeline_id: Some(id(pipeline.as_str())),
        r#ref: None,
    };

    // when
    let mut client = PipelineServiceClient::new(app.channel.clone());
    let manual = client.run_pipeline(with_token(&session, request)).await;
    let api = app.run_pipeline(owner, &pipeline).await.unwrap();

    // then
    let manual = manual.unwrap().into_inner().job.unwrap();
    assert_eq!(manual.origin(), JobOrigin::Manual);
    assert_eq!(api.origin(), JobOrigin::Api);
}
