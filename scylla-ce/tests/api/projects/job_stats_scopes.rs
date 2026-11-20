//! The tests of the figures of the jobs for the dashboard: the groups and the previous period,
//! the projects that the caller sees, and the bad requests.

use scylla_proto::pipelines::v1::job_group::Key;
use scylla_proto::pipelines::v1::{
    GetJobStatsRequest, JobOrigin, JobStatsBucket, JobStatsGroup, JobStatsOrder,
};
use scylla_testkit::{id, Role};
use sqlx::PgPool;

use super::job_stats::{created, job, outcomes, place, request, DAY, HOUR, NOV_2};
use crate::{app, assert_invalid_field};

/// The groups come in the order of the request, at most the limit, each with its figures of the
/// previous period. The origins filter the jobs.
#[sqlx::test(migrations = false)]
async fn get_job_stats_orders_the_groups_and_compares_them(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let ended = (Some(1), Some(2));
    for status in ["failed", "failed", "completed"] {
        job(&app, &place, 0, status, NOV_2 + HOUR, ended).await;
    }
    for status in ["orphaned", "completed"] {
        job(&app, &place, 1, status, NOV_2 + HOUR, ended).await;
    }
    for _ in 0..3 {
        job(&app, &place, 2, "completed", NOV_2 + HOUR, ended).await;
    }
    job(&app, &place, 0, "failed", NOV_2 - HOUR, ended).await;
    let request = GetJobStatsRequest {
        group_by: JobStatsGroup::Pipeline.into(),
        group_order: JobStatsOrder::Failed.into(),
        group_limit: 2,
        compare: true,
        ..request(place.workspace, NOV_2, NOV_2 + DAY)
    };
    let cron = GetJobStatsRequest {
        origins: vec![JobOrigin::Cron.into()],
        ..request.clone()
    };

    // when
    let stats = app.get_job_stats(place.owner, request).await.unwrap();
    let none = app.get_job_stats(place.owner, cron).await.unwrap();

    // then
    let keys: Vec<Key> = stats
        .groups
        .iter()
        .map(|group| group.key.clone().unwrap())
        .collect();
    let pipeline = |index: usize| Key::Id(id(place.pipelines[index].as_str()));
    assert_eq!(keys, vec![pipeline(0), pipeline(1)]);
    assert_eq!(
        stats.groups[0].figures.unwrap().outcomes,
        outcomes(1, 2, 0, 0)
    );
    assert_eq!(created(&stats.groups[0].previous), 1);
    assert_eq!(created(&stats.groups[1].previous), 0);
    assert_eq!(created(&stats.totals), 8);
    assert_eq!(created(&stats.previous), 1);
    assert_eq!(created(&none.totals), 0);
    assert!(none.groups.is_empty());
}

/// A member with a role on one project counts only the jobs of this project, and a filter on
/// another project counts no job. A user out of the workspace gets figures at 0, not a refusal.
#[sqlx::test(migrations = false)]
async fn get_job_stats_counts_only_the_projects_that_the_caller_sees(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let ended = (Some(1), Some(2));
    job(&app, &place, 0, "completed", NOV_2 + HOUR, ended).await;
    job(&app, &place, 2, "completed", NOV_2 + HOUR, ended).await;
    let (viewer, stranger) = (app.user().await, app.user().await);
    app.join_workspace(place.owner, viewer).await;
    let role = Role::on("project", &place.projects[0], "viewer");
    app.grant_role(place.owner, role, &viewer.to_string())
        .await
        .unwrap();
    let all = GetJobStatsRequest {
        group_by: JobStatsGroup::Project.into(),
        ..request(place.workspace, NOV_2, NOV_2 + DAY)
    };
    let other = GetJobStatsRequest {
        project_ids: vec![id(place.projects[1].as_str())],
        ..all.clone()
    };

    // when
    let owned = app.get_job_stats(place.owner, all.clone()).await.unwrap();
    let seen = app.get_job_stats(viewer, all.clone()).await.unwrap();
    let hidden = app.get_job_stats(viewer, other).await.unwrap();
    let none = app.get_job_stats(stranger, all).await.unwrap();

    // then
    assert_eq!(created(&owned.totals), 2);
    assert_eq!(created(&seen.totals), 1);
    let alpha = Key::Id(id(place.projects[0].as_str()));
    assert_eq!(seen.groups.len(), 1);
    assert_eq!(seen.groups[0].key, Some(alpha));
    assert_eq!(created(&hidden.totals), 0);
    assert_eq!(created(&none.totals), 0);
}

/// A request with no start, an end that is not after the start, a period of more than 400 days,
/// more than 1000 buckets, a group limit above 100, or an unset origin gives INVALID_ARGUMENT on
/// its field.
#[sqlx::test(migrations = false)]
async fn get_job_stats_refuses_a_bad_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let good = request(place.workspace, NOV_2, NOV_2 + DAY);
    let cases = [
        (
            "from",
            GetJobStatsRequest {
                from: None,
                ..good.clone()
            },
        ),
        ("to", request(place.workspace, NOV_2, NOV_2)),
        ("to", request(place.workspace, NOV_2 - 401 * DAY, NOV_2)),
        (
            "bucket",
            GetJobStatsRequest {
                bucket: JobStatsBucket::Hour.into(),
                ..request(place.workspace, NOV_2 - 50 * DAY, NOV_2)
            },
        ),
        (
            "group_limit",
            GetJobStatsRequest {
                group_limit: 101,
                ..good.clone()
            },
        ),
        (
            "origins",
            GetJobStatsRequest {
                origins: vec![JobOrigin::Unspecified.into()],
                ..good.clone()
            },
        ),
    ];

    for (field, request) in cases {
        // when
        let status = app
            .get_job_stats(place.owner, request)
            .await
            .expect_err(field);

        // then
        assert_invalid_field(&status, field);
    }
}
