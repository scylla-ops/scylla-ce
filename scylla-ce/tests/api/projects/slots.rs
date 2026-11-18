//! The tests of `GetSlotUsage`: the busy slots of the jobs in a period and in its buckets, the
//! slots of the connected agents, one agent, who can read them, and the bad requests.

use prost_types::Timestamp;
use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_proto::agents::v1::{GetSlotUsageRequest, SlotFigures};
use scylla_proto::pipelines::v1::JobStatsBucket;
use scylla_testkit::{id, TestApp};
use sqlx::PgPool;
use tonic::Code;

use super::job_stats::{job, place, DAY, HOUR, NOV_2};
use super::runs::agent;
use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Returns the request of the busy slots of a workspace from `from` to `to`, in seconds since
/// 1970.
fn request(workspace: &str, from: i64, to: i64) -> GetSlotUsageRequest {
    let at = |seconds| Some(Timestamp { seconds, nanos: 0 });
    GetSlotUsageRequest {
        workspace_id: Some(id(workspace)),
        from: at(from),
        to: at(to),
        ..GetSlotUsageRequest::default()
    }
}

/// Gives a job to an agent.
async fn give(app: &TestApp, job: &str, agent: &str) {
    sqlx::query!(
        "UPDATE jobs SET agent_id = $2 WHERE id = $1",
        JobId::parse(job).unwrap().to_db(),
        AppId::parse(agent).unwrap().to_db(),
    )
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Returns the mean and the most of busy slots.
fn busy(figures: Option<SlotFigures>) -> (f64, u32) {
    let figures = figures.expect("figures");
    let mean = (figures.busy_slots_avg * 1_000.0).round() / 1_000.0;
    (mean, figures.busy_slots_max)
}

/// Each job that started holds one slot from its start to its end, and a job that runs holds it
/// up to the end of the period. A pending job and a job before the period hold no slot.
#[sqlx::test(migrations = false)]
async fn get_slot_usage_adds_the_busy_time_of_each_hour(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    job(&app, &place, 0, "completed", NOV_2, (Some(0), Some(HOUR))).await;
    let half = (Some(0), Some(HOUR));
    job(&app, &place, 1, "failed", NOV_2 + HOUR / 2, half).await;
    let late = NOV_2 + 2 * HOUR + HOUR / 2;
    job(&app, &place, 2, "running", late, (Some(0), None)).await;
    job(&app, &place, 2, "pending", NOV_2, (None, None)).await;
    job(
        &app,
        &place,
        0,
        "completed",
        NOV_2 - DAY,
        (Some(1), Some(900)),
    )
    .await;
    let workspace = place.workspace.to_string();
    let request = GetSlotUsageRequest {
        bucket: JobStatsBucket::Hour.into(),
        ..request(&workspace, NOV_2, NOV_2 + 3 * HOUR)
    };

    // when
    let usage = app.get_slot_usage(place.owner, request).await.unwrap();

    // then
    assert_eq!(busy(usage.totals), (0.833, 2));
    let starts: Vec<i64> = usage
        .buckets
        .iter()
        .map(|bucket| bucket.start.unwrap().seconds)
        .collect();
    assert_eq!(starts, vec![NOV_2, NOV_2 + HOUR, NOV_2 + 2 * HOUR]);
    let figures: Vec<(f64, u32)> = usage
        .buckets
        .iter()
        .map(|bucket| busy(bucket.figures))
        .collect();
    assert_eq!(figures, vec![(1.5, 2), (0.5, 1), (0.5, 1)]);
    assert_eq!((usage.capacity, usage.time_zone.as_str()), (0, "UTC"));
}

/// The capacity is the sum of the slots of the connected agents. With an agent, the busy slots
/// and the capacity are the ones of this agent.
#[sqlx::test(migrations = false)]
async fn get_slot_usage_gives_the_slots_of_the_connected_agents(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let (owner, workspace) = (place.owner, place.workspace);
    let first = agent(&app, owner, workspace, false, 4, "").await;
    let second = agent(&app, owner, workspace, false, 2, "").await;
    let long = job(&app, &place, 0, "completed", NOV_2, (Some(0), Some(HOUR))).await;
    let short = job(
        &app,
        &place,
        1,
        "completed",
        NOV_2,
        (Some(0), Some(HOUR / 2)),
    )
    .await;
    give(&app, &long, &first.key.app).await;
    give(&app, &short, &second.key.app).await;
    let all = request(&workspace.to_string(), NOV_2, NOV_2 + HOUR);
    let one = GetSlotUsageRequest {
        agent_id: Some(id(first.key.app.as_str())),
        ..all.clone()
    };

    // when
    let of_all = app.get_slot_usage(owner, all).await.unwrap();
    let of_one = app.get_slot_usage(owner, one).await.unwrap();

    // then
    assert_eq!((busy(of_all.totals), of_all.capacity), ((1.5, 2), 6));
    assert_eq!((busy(of_one.totals), of_one.capacity), ((1.0, 1), 4));
    assert!(of_all.buckets.is_empty());
}

/// A member of the workspace reads the busy slots. A stranger and an unknown workspace get
/// `PERMISSION_DENIED`.
#[sqlx::test(migrations = false)]
async fn get_slot_usage_needs_a_reader(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace: WorkspaceId = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let workspace = workspace.to_string();

    // when
    let by_member = app
        .get_slot_usage(member, request(&workspace, NOV_2, NOV_2 + DAY))
        .await;
    let by_stranger = app
        .get_slot_usage(stranger, request(&workspace, NOV_2, NOV_2 + DAY))
        .await;
    let unknown = app
        .get_slot_usage(owner, request(UNKNOWN_ID, NOV_2, NOV_2 + DAY))
        .await;

    // then
    assert!(by_member.is_ok());
    assert_code(by_stranger, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
}

/// A request with no workspace, no start, an end that is not after the start, or more than 1000
/// buckets gives INVALID_ARGUMENT on its field.
#[sqlx::test(migrations = false)]
async fn get_slot_usage_refuses_a_bad_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await.to_string();
    let good = request(&workspace, NOV_2, NOV_2 + DAY);
    let cases = [
        (
            "workspace_id",
            GetSlotUsageRequest {
                workspace_id: None,
                ..good.clone()
            },
        ),
        (
            "from",
            GetSlotUsageRequest {
                from: None,
                ..good.clone()
            },
        ),
        ("to", request(&workspace, NOV_2, NOV_2)),
        (
            "bucket",
            GetSlotUsageRequest {
                bucket: JobStatsBucket::Hour.into(),
                ..request(&workspace, NOV_2 - 50 * DAY, NOV_2)
            },
        ),
    ];

    for (field, request) in cases {
        // when
        let status = app.get_slot_usage(owner, request).await.expect_err(field);

        // then
        assert_invalid_field(&status, field);
    }
}
