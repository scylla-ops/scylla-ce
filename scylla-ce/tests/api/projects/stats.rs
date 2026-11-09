//! The tests of the job stats of the agents: the ends of the jobs on each day, their durations,
//! the agents of a workspace, and who can read them.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use prost_types::Timestamp;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::agents::v1::{EventKind, JobOutcomes};
use scylla_proto::pipelines::v1::JobStatus;
use scylla_testkit::{exec_node, text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use super::runs::{agent, pipeline, Agent};
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Runs a pipeline and gives its job to `agent`, which ends it with `end`. Returns the job.
async fn ended_job(
    app: &TestApp,
    owner: ActorId,
    pipeline: &str,
    agent: &mut Agent,
    end: EventKind,
) -> String {
    app.run_pipeline(owner, pipeline).await.unwrap();
    let job = text(&agent.dispatch().await.job_id).to_owned();
    agent.event(&job, EventKind::JobStarted, "").await;
    if end == EventKind::JobCompleted {
        agent.event(&job, EventKind::NodeStarted, "build").await;
        agent.event(&job, EventKind::NodeCompleted, "build").await;
    }
    agent.event(&job, end, "").await;
    for _ in 0..50 {
        let status = app.get_job(owner, &job).await.unwrap().status;
        if status != JobStatus::Pending as i32 && status != JobStatus::Running as i32 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    job
}

/// Sets the creation time of a job to `days` days ago, and its run to `seconds` seconds.
async fn move_job(app: &TestApp, job: &str, days: i32, seconds: i32) {
    sqlx::query!(
        "UPDATE jobs SET created_at = now() - make_interval(days => $2),
                         started_at = now() - make_interval(secs => $3), finished_at = now()
         WHERE id = $1",
        JobId::parse(job).unwrap().to_db(),
        days,
        f64::from(seconds),
    )
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Sets the creation time of an App to `days` days ago.
async fn move_app(app: &TestApp, id: &str, days: i32) {
    sqlx::query!(
        "UPDATE apps SET created_at = now() - make_interval(days => $2) WHERE id = $1",
        AppId::parse(id).unwrap().to_db(),
        days,
    )
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Returns the time `days` days ago, or later for a negative number.
fn days_ago(days: i64) -> Option<Timestamp> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let seconds = i64::try_from(now.as_secs()).unwrap() - days * 86_400;
    Some(Timestamp { seconds, nanos: 0 })
}

/// Returns outcomes with a number of completed and failed jobs.
fn ends(completed: u32, failed: u32) -> Option<JobOutcomes> {
    Some(JobOutcomes {
        completed,
        failed,
        cancelled: 0,
        orphaned: 0,
    })
}

/// The stats count the jobs that ended in the 30 days on the day of their creation, with the
/// median and the 95th percentile of their run. A held job and an older job are in no count.
#[sqlx::test(migrations = false)]
async fn get_agent_stats_counts_the_ends_of_each_day(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut busy = agent(&app, owner, workspace, true, 4, "").await;
    let done = ended_job(&app, owner, &pipeline, &mut busy, EventKind::JobCompleted).await;
    let failed = ended_job(&app, owner, &pipeline, &mut busy, EventKind::JobFailed).await;
    let old = ended_job(&app, owner, &pipeline, &mut busy, EventKind::JobFailed).await;
    app.run_pipeline(owner, &pipeline).await.unwrap();
    busy.dispatch().await;
    move_job(&app, &done, 0, 10).await;
    move_job(&app, &failed, 2, 30).await;
    move_job(&app, &old, 40, 5).await;

    // when
    let stats = app
        .get_agent_stats(owner, &busy.key.app, None, None)
        .await
        .unwrap();

    // then
    assert_eq!(text(&stats.agent_id), busy.key.app);
    assert_eq!(stats.outcomes, ends(1, 1));
    let seconds = |duration: Option<prost_types::Duration>| {
        let duration = duration.unwrap();
        duration.seconds as f64 + f64::from(duration.nanos) / 1e9
    };
    assert!((seconds(stats.median_duration) - 20.0).abs() < 0.001);
    assert!((seconds(stats.p95_duration) - 29.0).abs() < 0.001);
    assert_eq!(stats.days.len(), 30);
    assert_eq!(stats.days[29].outcomes, ends(1, 0));
    assert_eq!(stats.days[27].outcomes, ends(0, 1));
    let busy_days = stats.days.iter().filter(|day| day.outcomes != ends(0, 0));
    assert_eq!(busy_days.count(), 2);
    let today = stats.days[29].day.unwrap();
    assert_eq!((today.seconds % 86_400, today.nanos), (0, 0));
}

/// A period counts only its own days, with the median of their jobs. The server moves its start
/// to the creation of the agent and its end to today, and refuses a period that ends before it
/// starts.
#[sqlx::test(migrations = false)]
async fn get_agent_stats_counts_the_jobs_of_a_period(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, pipeline) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut busy = agent(&app, owner, workspace, true, 4, "").await;
    let today = ended_job(&app, owner, &pipeline, &mut busy, EventKind::JobCompleted).await;
    let recent = ended_job(&app, owner, &pipeline, &mut busy, EventKind::JobFailed).await;
    let old = ended_job(&app, owner, &pipeline, &mut busy, EventKind::JobCompleted).await;
    let early = ended_job(&app, owner, &pipeline, &mut busy, EventKind::JobFailed).await;
    move_job(&app, &today, 0, 10).await;
    move_job(&app, &recent, 10, 30).await;
    move_job(&app, &old, 40, 5).await;
    move_job(&app, &early, 70, 5).await;
    move_app(&app, &busy.key.app, 60).await;
    let agent = busy.key.app.as_str();

    // when
    let middle = app
        .get_agent_stats(owner, agent, days_ago(45), days_ago(5))
        .await;
    let wide = app
        .get_agent_stats(owner, agent, days_ago(90), days_ago(-3))
        .await;
    let reversed = app
        .get_agent_stats(owner, agent, days_ago(5), days_ago(45))
        .await;

    // then
    let middle = middle.unwrap();
    assert_eq!(middle.outcomes, ends(1, 1));
    assert_eq!(middle.days.len(), 41);
    assert_eq!(middle.days[5].outcomes, ends(1, 0));
    assert_eq!(middle.days[35].outcomes, ends(0, 1));
    let median = middle.median_duration.unwrap();
    assert_eq!((median.seconds, median.nanos), (17, 500_000_000));
    let wide = wide.unwrap();
    assert_eq!(wide.outcomes, ends(2, 1));
    assert_eq!(wide.days.len(), 61);
    assert_eq!(wide.days[60].outcomes, ends(1, 0));
    assert_invalid_field(&reversed.unwrap_err(), "to");
}

/// The list has each agent of the workspace with a job, and an agent with no job gets stats at
/// 0 from `GetAgentStats`.
#[sqlx::test(migrations = false)]
async fn list_agent_stats_gives_each_agent_with_a_job(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, workspace, _, ours) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut busy = agent(&app, owner, workspace, true, 2, "").await;
    ended_job(&app, owner, &ours, &mut busy, EventKind::JobFailed).await;
    let idle = agent(&app, owner, workspace, true, 2, "").await;
    let (other, elsewhere, _, theirs) = pipeline(&app, vec![exec_node("build", &[])]).await;
    let mut foreign = agent(&app, other, elsewhere, true, 2, "").await;
    ended_job(&app, other, &theirs, &mut foreign, EventKind::JobFailed).await;

    // when
    let listed = app.list_agent_stats(owner, workspace).await.unwrap();
    let empty = app
        .get_agent_stats(owner, &idle.key.app, None, None)
        .await
        .unwrap();

    // then
    let agents: Vec<_> = listed.iter().map(|stats| text(&stats.agent_id)).collect();
    assert_eq!(agents, vec![busy.key.app.as_str()]);
    assert_eq!(listed[0].outcomes, ends(0, 1));
    assert_eq!(empty.outcomes, ends(0, 0));
    assert!(empty.median_duration.is_none() && empty.p95_duration.is_none());
    assert_eq!(empty.days.len(), 30);
}

/// The readers of the workspace read the stats. Another user and an unknown agent get a
/// refusal.
#[sqlx::test(migrations = false)]
async fn the_agent_stats_need_a_reader(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let (key, _) = app.app_in(owner, workspace, true).await;

    // when
    let by_member = app.get_agent_stats(member, &key.app, None, None).await;
    let listed = app.list_agent_stats(member, workspace).await;
    let by_stranger = app.get_agent_stats(stranger, &key.app, None, None).await;
    let list_by_stranger = app.list_agent_stats(stranger, workspace).await;
    let unknown = app.get_agent_stats(owner, UNKNOWN_ID, None, None).await;

    // then
    assert!(by_member.is_ok() && listed.is_ok());
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(list_by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
}
