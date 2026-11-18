//! The tests of `GetAgentMetrics`: the samples of the machine of an agent in a period and in its
//! buckets, an agent with no sample, who can read them, and the bad requests.

use std::time::{SystemTime, UNIX_EPOCH};

use prost_types::Timestamp;
use scylla_domain::identity::apps::AppId;
use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::agents::v1::{GetAgentMetricsRequest, MetricsFigures};
use scylla_proto::pipelines::v1::JobStatsBucket;
use scylla_testkit::{id, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// One hour, in seconds.
const HOUR: i64 = 3_600;

/// Returns the start of the hour two hours before now, in seconds since 1970.
fn two_hours_ago() -> i64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let now = i64::try_from(now.as_secs()).unwrap();
    now - now % HOUR - 2 * HOUR
}

/// Returns the request of the samples of an agent from `from` to `to`, in seconds since 1970,
/// in hours.
fn request(agent: &str, from: i64, to: i64) -> GetAgentMetricsRequest {
    let at = |seconds| Some(Timestamp { seconds, nanos: 0 });
    GetAgentMetricsRequest {
        agent_id: Some(id(agent)),
        from: at(from),
        to: at(to),
        bucket: JobStatsBucket::Hour.into(),
    }
}

/// Records a sample of an agent at `at`, in seconds since 1970.
async fn sample(app: &TestApp, agent: &str, at: i64, cpu_percent: f32, memory: i64) {
    sqlx::query!(
        "INSERT INTO agent_samples (agent_id, at, cpu_percent, memory_used_bytes)
         VALUES ($1, to_timestamp($2::float8), $3, $4)",
        AppId::parse(agent).unwrap().to_db(),
        at as f64,
        cpu_percent,
        memory,
    )
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Returns figures of samples.
fn figures(samples: u32, cpu: (f32, f32), memory: (u64, u64)) -> Option<MetricsFigures> {
    Some(MetricsFigures {
        samples,
        cpu_avg_percent: cpu.0,
        cpu_max_percent: cpu.1,
        memory_avg_bytes: memory.0,
        memory_max_bytes: memory.1,
    })
}

/// The totals and each hour give the number of samples, the mean and the most of the CPU use
/// and of the memory. A sample before the period is in no count.
#[sqlx::test(migrations = false)]
async fn get_agent_metrics_adds_the_samples_of_each_hour(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (key, _) = app.app_in(user, workspace, true).await;
    let from = two_hours_ago();
    sample(&app, &key.app, from - 600, 99.0, 900).await;
    sample(&app, &key.app, from + 600, 10.0, 100).await;
    sample(&app, &key.app, from + 1_200, 30.0, 300).await;
    sample(&app, &key.app, from + HOUR + 600, 50.0, 500).await;

    // when
    let metrics = app
        .get_agent_metrics(user, request(&key.app, from, from + 2 * HOUR))
        .await
        .unwrap();

    // then
    assert_eq!(metrics.totals, figures(3, (30.0, 50.0), (300, 500)));
    let starts: Vec<i64> = metrics
        .buckets
        .iter()
        .map(|bucket| bucket.start.unwrap().seconds)
        .collect();
    assert_eq!(starts, vec![from, from + HOUR]);
    assert_eq!(
        metrics.buckets[0].figures,
        figures(2, (20.0, 30.0), (200, 300))
    );
    assert_eq!(
        metrics.buckets[1].figures,
        figures(1, (50.0, 50.0), (500, 500))
    );
    assert_eq!(metrics.time_zone, "UTC");
}

/// An agent that sent no sample gets figures of 0 in the totals and in each bucket.
#[sqlx::test(migrations = false)]
async fn get_agent_metrics_of_an_agent_with_no_sample(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (key, _) = app.app_in(user, workspace, true).await;
    let from = two_hours_ago();

    // when
    let metrics = app
        .get_agent_metrics(user, request(&key.app, from, from + 2 * HOUR))
        .await
        .unwrap();

    // then
    let zero = figures(0, (0.0, 0.0), (0, 0));
    assert_eq!(metrics.totals, zero);
    assert_eq!(metrics.buckets.len(), 2);
    assert!(metrics.buckets.iter().all(|bucket| bucket.figures == zero));
}

/// A member of the workspace reads the samples of an agent. A stranger and an unknown App get
/// `PERMISSION_DENIED`.
#[sqlx::test(migrations = false)]
async fn get_agent_metrics_needs_a_reader(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let (key, _) = app.app_in(owner, workspace, true).await;
    let from = two_hours_ago();

    // when
    let by_member = app
        .get_agent_metrics(member, request(&key.app, from, from + HOUR))
        .await;
    let by_stranger = app
        .get_agent_metrics(stranger, request(&key.app, from, from + HOUR))
        .await;
    let unknown = app
        .get_agent_metrics(owner, request(UNKNOWN_ID, from, from + HOUR))
        .await;

    // then
    assert!(by_member.is_ok());
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
}

/// A request with no agent, no start, an end that is not after the start, or more than 1000
/// buckets gives INVALID_ARGUMENT on its field.
#[sqlx::test(migrations = false)]
async fn get_agent_metrics_refuses_a_bad_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (key, _) = app.app_in(user, workspace, true).await;
    let from = two_hours_ago();
    let good = request(&key.app, from, from + HOUR);
    let cases = [
        (
            "agent_id",
            GetAgentMetricsRequest {
                agent_id: None,
                ..good.clone()
            },
        ),
        (
            "from",
            GetAgentMetricsRequest {
                from: None,
                ..good.clone()
            },
        ),
        ("to", request(&key.app, from, from)),
        ("bucket", request(&key.app, from - 50 * 24 * HOUR, from)),
    ];

    for (field, request) in cases {
        // when
        let status = app.get_agent_metrics(user, request).await.expect_err(field);

        // then
        assert_invalid_field(&status, field);
    }
}
