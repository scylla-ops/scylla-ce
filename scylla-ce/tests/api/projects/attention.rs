//! The tests of the open items of the dashboard ("Needs you"): the jobs and the fires that stay
//! open until a later good run, the new items and the pages. The kinds that a caller reads, the
//! approvals, the agents and the bad requests are in `attention_access.rs`.

use std::time::{SystemTime, UNIX_EPOCH};

use prost_types::Timestamp;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::triggers::TriggerId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_proto::common::v1::PageRequest;
use scylla_proto::dashboards::v1::attention_item::Subject;
use scylla_proto::dashboards::v1::{
    AttentionKind, ListAttentionItemsRequest, ListAttentionItemsResponse,
};
use scylla_testkit::{id, text, TestApp};
use sqlx::PgPool;

use super::job_stats::{place, Place};
use crate::app;

/// Returns the request of the open items of a workspace.
pub(super) fn request(workspace: WorkspaceId) -> ListAttentionItemsRequest {
    ListAttentionItemsRequest {
        workspace_id: Some(id(workspace.to_string())),
        ..ListAttentionItemsRequest::default()
    }
}

/// Starts a job of a pipeline, then ends it `ago` minutes ago with a state, on a branch if one is
/// given. Returns the id of the job.
pub(super) async fn ended(
    app: &TestApp,
    place: &Place,
    pipeline: usize,
    status: &str,
    ago: i32,
    branch: Option<&str>,
) -> String {
    let job = app.run_pipeline(place.owner, &place.pipelines[pipeline]);
    let job = text(&job.await.unwrap().id).to_owned();
    let row = JobId::parse(&job).unwrap().to_db();
    sqlx::query!(
        "UPDATE jobs SET status = $2, started_at = now() - make_interval(mins => $3::int4 + 1),
                finished_at = now() - make_interval(mins => $3::int4)
         WHERE id = $1",
        row,
        status,
        ago,
    )
    .execute(&app.pool)
    .await
    .unwrap();
    if let Some(branch) = branch {
        sqlx::query!(
            "INSERT INTO job_sources (job_id, pipeline_id, provider, repository, url, sha, branch,
                                     event, automatic)
             SELECT id, pipeline_id, 'github', 'acme/app', 'https://github.com/acme/app', 'abc',
                    $2, 'manual', false
             FROM jobs WHERE id = $1",
            row,
            branch,
        )
        .execute(&app.pool)
        .await
        .unwrap();
    }
    job
}

/// Records a fire of a trigger due `ago` minutes ago with a result: `ok`, `error` or `missed`. A
/// fire starts 2 seconds after its due time.
pub(super) async fn fire(app: &TestApp, trigger: &str, ago: i32, status: &str) {
    sqlx::query!(
        "INSERT INTO trigger_fires (id, trigger_id, pipeline_id, project_id, workspace_id, source,
                                    due_at, fired_at, status, error)
         SELECT gen_random_uuid(), t.id, t.pipeline_id, t.project_id, t.workspace_id, 'schedule',
                now() - make_interval(mins => $2::int4),
                CASE WHEN $3 <> 'missed'
                     THEN now() - make_interval(mins => $2::int4) + interval '2 seconds' END,
                $3, CASE WHEN $3 = 'error' THEN 'no agent' END
         FROM triggers t WHERE t.id = $1",
        TriggerId::parse(trigger).unwrap().to_db(),
        ago,
        status,
    )
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Creates a cron trigger of a pipeline, and returns its id.
pub(super) async fn cron(app: &TestApp, place: &Place, pipeline: usize, name: &str) -> String {
    let created = app
        .create_trigger(
            place.owner,
            &place.pipelines[pipeline],
            name,
            Some("0 3 * * *"),
            Vec::new(),
        )
        .await
        .unwrap();
    text(&created.trigger.unwrap().id).to_owned()
}

/// Returns the open count of each kind of a response.
pub(super) fn counts(response: &ListAttentionItemsResponse) -> Vec<(AttentionKind, u32)> {
    let counts = response.counts.iter();
    counts
        .map(|count| (count.kind(), count.open_count))
        .collect()
}

/// Returns the kind, the count and the id of the subject of each item of a response.
pub(super) fn items(response: &ListAttentionItemsResponse) -> Vec<(AttentionKind, u32, String)> {
    let subject = |subject: &Subject| match subject {
        Subject::Job(job) => text(&job.id).to_owned(),
        Subject::Fire(fire) => text(&fire.id).to_owned(),
        Subject::Agent(agent) => text(&agent.id).to_owned(),
    };
    let items = response.items.iter();
    items
        .map(|item| {
            (
                item.kind(),
                item.count,
                subject(item.subject.as_ref().unwrap()),
            )
        })
        .collect()
}

/// Returns the seconds since 1970 of a time `ago` minutes before now.
fn minutes_ago(ago: i64) -> i64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    now.as_secs() as i64 - ago * 60
}

/// A failed or orphaned job stays open until a later job of the same pipeline and branch
/// completes. The jobs of one pipeline, branch and kind make one item, with the newest job.
#[sqlx::test(migrations = false)]
async fn a_failed_job_stays_open_until_a_later_job_of_its_branch_completes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    ended(&app, &place, 0, "failed", 60, Some("main")).await;
    let second = ended(&app, &place, 0, "failed", 50, Some("main")).await;
    let orphan = ended(&app, &place, 0, "orphaned", 40, Some("dev")).await;
    ended(&app, &place, 1, "failed", 45, Some("main")).await;
    ended(&app, &place, 1, "completed", 30, Some("main")).await;
    ended(&app, &place, 2, "completed", 70, None).await;
    let last = ended(&app, &place, 2, "failed", 20, None).await;
    let jobs = ListAttentionItemsRequest {
        kinds: vec![
            AttentionKind::JobFailed.into(),
            AttentionKind::JobOrphaned.into(),
        ],
        ..request(place.workspace)
    };

    // when
    let open = app.list_attention_items(place.owner, jobs).await.unwrap();

    // then
    let expected = vec![
        (AttentionKind::JobFailed, 1, last),
        (AttentionKind::JobOrphaned, 1, orphan),
        (AttentionKind::JobFailed, 2, second),
    ];
    assert_eq!(items(&open), expected);
    let expected = vec![
        (AttentionKind::JobFailed, 2),
        (AttentionKind::JobOrphaned, 1),
    ];
    assert_eq!(counts(&open), expected);
    assert_eq!((open.open_count, open.new_count), (3, 0));
    assert_eq!(text(&open.items[2].pipeline_id), place.pipelines[0]);
    let cleared = open.last_cleared_at.unwrap().seconds;
    assert!((cleared - minutes_ago(30)).abs() < 60);
    let Some(Subject::Job(job)) = &open.items[2].subject else {
        panic!("a job")
    };
    assert_eq!(job.branch, "main");
}

/// A failed fire and a missed time stay open until the next good fire of the trigger.
#[sqlx::test(migrations = false)]
async fn a_bad_fire_stays_open_until_the_next_good_fire_of_its_trigger(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let (nightly, hourly) = (
        cron(&app, &place, 0, "nightly").await,
        cron(&app, &place, 2, "hourly").await,
    );
    fire(&app, &nightly, 70, "ok").await;
    for (ago, status) in [(60, "error"), (50, "missed"), (40, "error")] {
        fire(&app, &nightly, ago, status).await;
    }
    fire(&app, &hourly, 60, "missed").await;
    fire(&app, &hourly, 30, "ok").await;
    let fires = ListAttentionItemsRequest {
        kinds: vec![
            AttentionKind::FireFailed.into(),
            AttentionKind::FireMissed.into(),
        ],
        ..request(place.workspace)
    };

    // when
    let open = app.list_attention_items(place.owner, fires).await.unwrap();

    // then
    let found: Vec<_> = items(&open)
        .into_iter()
        .map(|(kind, count, _)| (kind, count))
        .collect();
    let expected = vec![
        (AttentionKind::FireFailed, 2),
        (AttentionKind::FireMissed, 1),
    ];
    assert_eq!(found, expected);
    let Some(Subject::Fire(fire)) = &open.items[0].subject else {
        panic!("a fire")
    };
    assert_eq!(text(&fire.trigger_id), nightly);
    assert_eq!(
        (fire.trigger_name.as_str(), fire.error.as_str()),
        ("nightly", "no agent")
    );
    assert_eq!(fire.schedule, "0 3 * * *");
    let expected = vec![
        (AttentionKind::FireFailed, 1),
        (AttentionKind::FireMissed, 1),
    ];
    assert_eq!(counts(&open), expected);
    let cleared = open.last_cleared_at.unwrap().seconds;
    assert!((cleared - minutes_ago(30)).abs() < 60);
}

/// The items after the last visit are new, and the key of a page gives the next page. An item of
/// a job that ended more than 30 days ago is not open.
#[sqlx::test(migrations = false)]
async fn the_since_time_marks_the_new_items_and_a_key_gives_the_next_page(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    ended(&app, &place, 0, "failed", 31 * 24 * 60, None).await;
    let old = ended(&app, &place, 0, "failed", 60, Some("a")).await;
    let middle = ended(&app, &place, 1, "failed", 40, None).await;
    let new = ended(&app, &place, 2, "failed", 20, None).await;
    let first = ListAttentionItemsRequest {
        since: Some(Timestamp {
            seconds: minutes_ago(30),
            nanos: 0,
        }),
        page: Some(PageRequest {
            size: 2,
            token: String::new(),
        }),
        ..request(place.workspace)
    };

    // when
    let one = app
        .list_attention_items(place.owner, first.clone())
        .await
        .unwrap();
    let token = one.page.clone().unwrap().next_token;
    let next = ListAttentionItemsRequest {
        page: Some(PageRequest { size: 2, token }),
        ..first
    };
    let two = app.list_attention_items(place.owner, next).await.unwrap();

    // then
    let ids = |response: &ListAttentionItemsResponse| -> Vec<String> {
        items(response).into_iter().map(|(_, _, id)| id).collect()
    };
    assert_eq!(ids(&one), vec![new, middle]);
    assert_eq!(ids(&two), vec![old]);
    let marks: Vec<bool> = one.items.iter().map(|item| item.is_new).collect();
    assert_eq!(marks, vec![true, false]);
    assert_eq!((one.open_count, one.new_count), (3, 1));
    assert_eq!(two.page.unwrap().next_token, "");
}
