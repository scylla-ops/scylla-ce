//! The tests of the history of the fires of the triggers: the fires of a webhook and of a fire
//! now, the projects that the caller sees, the counts of the fires and of the missed times, and
//! the bad requests.

use prost_types::Timestamp;
use scylla_domain::projects::triggers::TriggerId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::pipelines::v1::{
    trigger, FireSource, FireStatsBucket, FireStatsGroup, FireStatus, GetTriggerFireStatsRequest,
    ListTriggerFiresRequest, TriggerFireFigures, TriggerKind,
};
use scylla_testkit::{id, text, Role, TestApp};
use sqlx::PgPool;

use super::job_stats::{place, DAY, HOUR, NOV_2};
use super::triggers::post;
use crate::{app, assert_invalid_field};

/// Returns the request of the fires of a workspace.
fn fires(workspace: WorkspaceId) -> ListTriggerFiresRequest {
    ListTriggerFiresRequest {
        workspace_id: Some(id(workspace.to_string())),
        ..ListTriggerFiresRequest::default()
    }
}

/// Returns the request of the counts of the fires of a workspace from `from` to `to`, in seconds
/// since 1970.
fn stats(workspace: WorkspaceId, from: i64, to: i64) -> GetTriggerFireStatsRequest {
    let at = |seconds| Some(Timestamp { seconds, nanos: 0 });
    GetTriggerFireStatsRequest {
        workspace_id: Some(id(workspace.to_string())),
        from: at(from),
        to: at(to),
        ..GetTriggerFireStatsRequest::default()
    }
}

/// Records a fire of a schedule due at `due` seconds since 1970 with a result: `ok`, `error` or
/// `missed`. A fire starts 2 seconds after its due time.
pub(super) async fn fire(app: &TestApp, trigger: &str, due: i64, status: &str) {
    sqlx::query!(
        "INSERT INTO trigger_fires (id, trigger_id, pipeline_id, project_id, workspace_id, source,
                                    due_at, fired_at, status, error)
         SELECT gen_random_uuid(), t.id, t.pipeline_id, t.project_id, t.workspace_id, 'schedule',
                to_timestamp($2::float8),
                CASE WHEN $3 <> 'missed' THEN to_timestamp($2::float8) + interval '2 seconds' END,
                $3, CASE WHEN $3 = 'error' THEN 'no agent' END
         FROM triggers t WHERE t.id = $1",
        TriggerId::parse(trigger).unwrap().to_db(),
        due as f64,
        status,
    )
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Returns the total, the fires that made a job, the failed fires and the missed times of counts.
fn counts(figures: &Option<TriggerFireFigures>) -> (u32, u32, u32, u32) {
    let figures = figures.as_ref().expect("figures");
    let (total, ok) = (figures.total, figures.ok);
    (total, ok, figures.failed_fires, figures.missed_times)
}

/// A signed webhook delivery and a fire now go in the history, newest first, with their jobs. A
/// bad signature writes nothing. A viewer of one project sees its fires only, and a user out of
/// the workspace sees none.
#[sqlx::test(migrations = false)]
async fn the_history_keeps_the_fires_of_the_webhooks_and_of_the_fires_now(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let owner = place.owner;
    let hook = app
        .create_trigger(owner, &place.pipelines[0], "hook", None, Vec::new())
        .await
        .unwrap();
    let Some(trigger::Kind::Webhook(webhook)) = hook.trigger.unwrap().kind else {
        panic!("a webhook trigger")
    };
    let nightly = app
        .create_trigger(
            owner,
            &place.pipelines[2],
            "nightly",
            Some("0 3 * * *"),
            Vec::new(),
        )
        .await
        .unwrap();
    let nightly = text(&nightly.trigger.unwrap().id).to_owned();
    let (viewer, stranger) = (app.user().await, app.user().await);
    app.join_workspace(owner, viewer).await;
    let role = Role::on("project", &place.projects[1], "viewer");
    app.grant_role(owner, role, &viewer.to_string())
        .await
        .unwrap();

    // when
    let (refused, _) = post(&app, &webhook.path, "{}", "bad", "key-1").await;
    let (accepted, _) = post(&app, &webhook.path, "{}", &hook.webhook_secret, "key-2").await;
    let job = app.fire_trigger_now(owner, &nightly).await.unwrap();
    let all = app.list_trigger_fires(owner, fires(place.workspace)).await;
    let manual = ListTriggerFiresRequest {
        sources: vec![FireSource::Manual.into()],
        kinds: vec![TriggerKind::Cron.into()],
        statuses: vec![FireStatus::Ok.into()],
        ..fires(place.workspace)
    };
    let manual = app.list_trigger_fires(owner, manual).await.unwrap();
    let seen = app.list_trigger_fires(viewer, fires(place.workspace)).await;
    let none = app
        .list_trigger_fires(stranger, fires(place.workspace))
        .await;

    // then
    assert_eq!((refused, accepted), (401, 202));
    let all = all.unwrap().fires;
    let sources: Vec<_> = all.iter().map(|one| one.source()).collect();
    assert_eq!(sources, vec![FireSource::Manual, FireSource::Webhook]);
    assert!(all
        .iter()
        .all(|one| one.status() == FireStatus::Ok && one.job_id.is_some()));
    assert_eq!(manual.fires.len(), 1);
    assert_eq!(manual.fires[0].job_id, job.id);
    assert_eq!(manual.fires[0].trigger_name, "nightly");
    let seen = seen.unwrap().fires;
    assert_eq!(seen.len(), 1);
    assert_eq!(text(&seen[0].trigger_id), nightly);
    assert!(none.unwrap().fires.is_empty());
}

/// The counts split the failed fires and the missed times, by day and by trigger: the trigger with
/// the most problems comes first. The fires out of the period do not count.
#[sqlx::test(migrations = false)]
async fn get_trigger_fire_stats_counts_the_fires_and_the_missed_times(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let mut ids = Vec::new();
    for (pipeline, name) in [(0, "alpha"), (2, "beta")] {
        let created = app
            .create_trigger(
                place.owner,
                &place.pipelines[pipeline],
                name,
                Some("0 * * * *"),
                Vec::new(),
            )
            .await
            .unwrap();
        ids.push(text(&created.trigger.unwrap().id).to_owned());
    }
    for (index, due, status) in [
        (0, NOV_2 - HOUR, "error"),
        (0, NOV_2 + HOUR, "ok"),
        (0, NOV_2 + 2 * HOUR, "ok"),
        (0, NOV_2 + 3 * HOUR, "missed"),
        (1, NOV_2 + DAY + HOUR, "ok"),
        (1, NOV_2 + DAY + 2 * HOUR, "error"),
        (1, NOV_2 + DAY + 3 * HOUR, "error"),
    ] {
        fire(&app, &ids[index], due, status).await;
    }
    let request = GetTriggerFireStatsRequest {
        bucket: FireStatsBucket::Day.into(),
        group_by: FireStatsGroup::Trigger.into(),
        ..stats(place.workspace, NOV_2, NOV_2 + 2 * DAY)
    };

    // when
    let found = app
        .get_trigger_fire_stats(place.owner, request)
        .await
        .unwrap();

    // then
    assert_eq!(counts(&found.totals), (6, 3, 2, 1));
    let delay = found.totals.unwrap().p50_delay.unwrap();
    assert_eq!((delay.seconds, delay.nanos), (2, 0));
    let days: Vec<_> = found
        .buckets
        .iter()
        .map(|day| counts(&day.figures))
        .collect();
    assert_eq!(days, vec![(3, 2, 0, 1), (3, 1, 2, 0)]);
    let groups: Vec<_> = found.groups.iter().map(|group| text(&group.id)).collect();
    assert_eq!(groups, vec![ids[1].as_str(), ids[0].as_str()]);
    assert_eq!(found.time_zone, "UTC");
}

/// A bad filter, an end that is not after the start, no start of the counts, or a group limit
/// above 100 gives INVALID_ARGUMENT on its field.
#[sqlx::test(migrations = false)]
async fn the_history_refuses_a_bad_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let at = |seconds| Some(Timestamp { seconds, nanos: 0 });
    let list = fires(place.workspace);
    let lists = [
        (
            "to",
            ListTriggerFiresRequest {
                from: at(NOV_2),
                to: at(NOV_2),
                ..list.clone()
            },
        ),
        (
            "statuses",
            ListTriggerFiresRequest {
                statuses: vec![0],
                ..list.clone()
            },
        ),
        (
            "sources",
            ListTriggerFiresRequest {
                sources: vec![0],
                ..list.clone()
            },
        ),
        (
            "kinds",
            ListTriggerFiresRequest {
                kinds: vec![0],
                ..list.clone()
            },
        ),
        (
            "trigger_ids",
            ListTriggerFiresRequest {
                trigger_ids: vec![id(place.pipelines[0].as_str()); 101],
                ..list.clone()
            },
        ),
    ];
    let good = stats(place.workspace, NOV_2, NOV_2 + DAY);
    let counts = [
        (
            "from",
            GetTriggerFireStatsRequest {
                from: None,
                ..good.clone()
            },
        ),
        (
            "group_limit",
            GetTriggerFireStatsRequest {
                group_limit: 101,
                ..good.clone()
            },
        ),
        (
            "sources",
            GetTriggerFireStatsRequest {
                sources: vec![0],
                ..good.clone()
            },
        ),
    ];

    // when
    let mut statuses = Vec::new();
    for (field, request) in lists {
        let status = app.list_trigger_fires(place.owner, request).await;
        statuses.push((field, status.expect_err(field)));
    }
    for (field, request) in counts {
        let status = app.get_trigger_fire_stats(place.owner, request).await;
        statuses.push((field, status.expect_err(field)));
    }

    // then
    for (field, status) in statuses {
        assert_invalid_field(&status, field);
    }
}
