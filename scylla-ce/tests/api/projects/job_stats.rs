//! The tests of the figures of the jobs for the dashboard: the totals and the buckets in the time
//! zone of the workspace. The groups, the scopes and the bad requests are in `job_stats_scopes.rs`.

use prost_types::Timestamp;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::{GetJobStatsRequest, JobFigures, JobOutcomes, JobStatsBucket};
use scylla_testkit::{exec_node, id, text, TestApp};
use sqlx::PgPool;

use crate::app;

/// 2026-11-02 00:00 UTC, in seconds since 1970.
pub(super) const NOV_2: i64 = 1_793_577_600;

/// One hour, in seconds.
pub(super) const HOUR: i64 = 3_600;

/// One day, in seconds.
pub(super) const DAY: i64 = 86_400;

/// A workspace with the project `alpha` of two pipelines and the project `beta` of one.
pub(super) struct Place {
    /// The owner of the workspace.
    pub(super) owner: ActorId,
    /// The workspace.
    pub(super) workspace: WorkspaceId,
    /// The ids of `alpha` and `beta`.
    pub(super) projects: [String; 2],
    /// The ids of the pipelines: two of `alpha`, then one of `beta`.
    pub(super) pipelines: [String; 3],
}

/// Creates a user and its two projects.
pub(super) async fn place(app: &TestApp) -> Place {
    let owner = app.user().await;
    let mut projects = Vec::new();
    let mut pipelines = Vec::new();
    for (name, count) in [("alpha", 2), ("beta", 1)] {
        let project = app.create_project(owner, name).await.unwrap();
        let project = text(&project.id).to_owned();
        for _ in 0..count {
            let pipeline = app.create_pipeline(owner, &project, vec![exec_node("build", &[])]);
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

/// Starts a job of a pipeline, then sets its state and its times: its creation at `created`
/// (seconds since 1970), and its start and its end `start` and `end` seconds after it. Returns
/// the id of the job.
pub(super) async fn job(
    app: &TestApp,
    place: &Place,
    pipeline: usize,
    status: &str,
    created: i64,
    times: (Option<i64>, Option<i64>),
) -> String {
    let job = app.run_pipeline(place.owner, &place.pipelines[pipeline]);
    let job = text(&job.await.unwrap().id).to_owned();
    sqlx::query!(
        "UPDATE jobs SET status = $2, created_at = to_timestamp($3::float8),
                started_at = to_timestamp($3::float8) + make_interval(secs => $4::float8),
                finished_at = to_timestamp($3::float8) + make_interval(secs => $5::float8)
         WHERE id = $1",
        JobId::parse(&job).unwrap().to_db(),
        status,
        created as f64,
        times.0.map(|seconds| seconds as f64),
        times.1.map(|seconds| seconds as f64),
    )
    .execute(&app.pool)
    .await
    .unwrap();
    job
}

/// Returns the request of the figures of a workspace from `from` to `to`, in seconds since 1970.
pub(super) fn request(workspace: WorkspaceId, from: i64, to: i64) -> GetJobStatsRequest {
    let at = |seconds| Some(Timestamp { seconds, nanos: 0 });
    GetJobStatsRequest {
        workspace_id: Some(id(workspace.to_string())),
        from: at(from),
        to: at(to),
        ..GetJobStatsRequest::default()
    }
}

/// Returns the number of seconds of a duration.
fn seconds(duration: Option<prost_types::Duration>) -> f64 {
    let duration = duration.expect("a duration");
    duration.seconds as f64 + f64::from(duration.nanos) / 1e9
}

/// Returns the outcomes of four counts.
pub(super) fn outcomes(
    completed: u32,
    failed: u32,
    cancelled: u32,
    orphaned: u32,
) -> Option<JobOutcomes> {
    Some(JobOutcomes {
        completed,
        failed,
        cancelled,
        orphaned,
    })
}

/// Returns the number of jobs of figures.
pub(super) fn created(figures: &Option<JobFigures>) -> u32 {
    figures.as_ref().expect("figures").created
}

/// The totals count each job created in the period, the ends of the jobs that ended, the run
/// times of the jobs that started and ended, and the waits for an agent from the creation, or
/// from the approval of a job from a fork. A job created before the period is in no count.
#[sqlx::test(migrations = false)]
async fn get_job_stats_counts_the_jobs_of_the_period(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let start = NOV_2 + HOUR;
    job(&app, &place, 0, "completed", start, (Some(10), Some(70))).await;
    job(&app, &place, 0, "failed", start, (Some(20), Some(140))).await;
    job(&app, &place, 1, "orphaned", start, (Some(30), Some(120))).await;
    job(&app, &place, 1, "cancelled", start, (None, Some(5))).await;
    job(&app, &place, 2, "pending", start, (None, None)).await;
    let approved = job(&app, &place, 2, "running", start, (Some(140), None)).await;
    job(
        &app,
        &place,
        2,
        "completed",
        NOV_2 - DAY,
        (Some(1), Some(900)),
    )
    .await;
    sqlx::query!(
        "UPDATE jobs SET approval = 'approved', approved_by = created_by,
                approved_at = created_at + interval '100 seconds'
         WHERE id = $1",
        JobId::parse(&approved).unwrap().to_db(),
    )
    .execute(&app.pool)
    .await
    .unwrap();

    // when
    let stats = app
        .get_job_stats(place.owner, request(place.workspace, NOV_2, NOV_2 + DAY))
        .await
        .unwrap();

    // then
    let totals = stats.totals.unwrap();
    assert_eq!(totals.created, 6);
    assert_eq!(totals.outcomes, outcomes(1, 1, 1, 1));
    assert!((seconds(totals.p50_duration) - 90.0).abs() < 0.001);
    assert!((seconds(totals.p95_duration) - 117.0).abs() < 0.001);
    assert!((seconds(totals.run_time) - 270.0).abs() < 0.001);
    assert!((seconds(totals.p50_queue) - 25.0).abs() < 0.001);
    assert!((seconds(totals.p95_queue) - 38.5).abs() < 0.001);
    assert!(stats.buckets.is_empty() && stats.groups.is_empty());
    assert_eq!(stats.previous, None);
    assert_eq!(stats.time_zone, "UTC");
}

/// The days of the buckets are the days of the time zone of the workspace, also the days with no
/// job. The first bucket starts before the period, and counts the jobs from its start.
#[sqlx::test(migrations = false)]
async fn get_job_stats_gives_the_days_of_the_workspace_time_zone(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let workspace = place.workspace.to_string();
    app.set_workspace_time_zone(place.owner, &workspace, "Europe/Paris")
        .await
        .unwrap();
    let ended = (Some(1), Some(2));
    job(&app, &place, 0, "completed", NOV_2 + 10 * HOUR, ended).await;
    job(
        &app,
        &place,
        0,
        "completed",
        NOV_2 + 22 * HOUR + 1_800,
        ended,
    )
    .await;
    job(&app, &place, 0, "failed", NOV_2 + 23 * HOUR + 1_800, ended).await;
    job(&app, &place, 0, "completed", NOV_2 + 3 * DAY - 3_000, ended).await;
    let request = GetJobStatsRequest {
        bucket: JobStatsBucket::Day.into(),
        ..request(
            place.workspace,
            NOV_2 + 12 * HOUR,
            NOV_2 + 3 * DAY + 12 * HOUR,
        )
    };

    // when
    let stats = app.get_job_stats(place.owner, request).await.unwrap();

    // then
    assert_eq!(stats.time_zone, "Europe/Paris");
    let starts: Vec<i64> = stats
        .buckets
        .iter()
        .map(|bucket| bucket.start.unwrap().seconds)
        .collect();
    let first = NOV_2 - HOUR;
    assert_eq!(
        starts,
        vec![first, first + DAY, first + 2 * DAY, first + 3 * DAY]
    );
    let counts: Vec<u32> = stats
        .buckets
        .iter()
        .map(|bucket| created(&bucket.figures))
        .collect();
    assert_eq!(counts, vec![1, 1, 0, 1]);
    assert_eq!(
        stats.buckets[1].figures.unwrap().outcomes,
        outcomes(0, 1, 0, 0)
    );
    assert_eq!(created(&stats.totals), 3);
}
