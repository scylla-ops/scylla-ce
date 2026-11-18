//! The tests of the list of the triggers of a workspace: the next fires in the time zone of the
//! workspace, the order and the pages, the recent results, the projects that the caller sees, and
//! the bad requests.

use std::time::{SystemTime, UNIX_EPOCH};

use prost_types::Duration;
use scylla_proto::common::v1::PageRequest;
use scylla_proto::pipelines::v1::{
    FireStatus, ListWorkspaceTriggersRequest, Trigger, TriggerKind, TriggerOrder,
};
use scylla_testkit::{id, text, Role, TestApp};
use sqlx::PgPool;

use super::job_stats::{place, Place, HOUR};
use super::trigger_fires::fire;
use crate::{app, assert_invalid_field};

/// Returns the request of the triggers of the workspace of a place, with the fires of the next
/// 24 hours.
fn request(place: &Place) -> ListWorkspaceTriggersRequest {
    ListWorkspaceTriggersRequest {
        workspace_id: Some(id(place.workspace.to_string())),
        upcoming: Some(Duration {
            seconds: 24 * HOUR,
            nanos: 0,
        }),
        ..ListWorkspaceTriggersRequest::default()
    }
}

/// Creates a trigger as the owner of a place, and returns it.
async fn trigger(
    app: &TestApp,
    place: &Place,
    pipeline: usize,
    name: &str,
    cron: Option<&str>,
) -> Trigger {
    let created = app
        .create_trigger(
            place.owner,
            &place.pipelines[pipeline],
            name,
            cron,
            Vec::new(),
        )
        .await
        .unwrap();
    created.trigger.unwrap()
}

/// Returns the hour in UTC of the next fire of a trigger.
fn utc_hour(trigger: &Trigger) -> i64 {
    let next = trigger.next_fire_at.as_ref().expect("a next fire");
    next.seconds.rem_euclid(24 * HOUR) / HOUR
}

/// A cron schedule follows the time zone of the workspace: a change of the zone moves the next
/// fire of 03:00 from 03:00 UTC to 03:00 in Paris.
#[sqlx::test(migrations = false)]
async fn a_change_of_the_time_zone_moves_the_next_fires(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let nightly = trigger(&app, &place, 0, "nightly", Some("0 3 * * *")).await;
    let workspace = place.workspace.to_string();

    // when
    app.set_workspace_time_zone(place.owner, &workspace, "Europe/Paris")
        .await
        .unwrap();
    let moved = app
        .get_trigger(place.owner, text(&nightly.id))
        .await
        .unwrap();

    // then
    assert_eq!(utc_hour(&nightly), 3);
    assert!([1, 2].contains(&utc_hour(&moved)), "{moved:?}");
}

/// The list gives the time zone, the fires of the window of each enabled cron trigger, the soonest
/// next fire first, and the pages in the order of the names.
#[sqlx::test(migrations = false)]
async fn list_workspace_triggers_gives_the_next_fires(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let workspace = place.workspace.to_string();
    app.set_workspace_time_zone(place.owner, &workspace, "Europe/Paris")
        .await
        .unwrap();
    trigger(&app, &place, 0, "nightly", Some("0 3 * * *")).await;
    trigger(&app, &place, 2, "hourly", Some("0 * * * *")).await;
    trigger(&app, &place, 1, "hook", None).await;
    let by_name = ListWorkspaceTriggersRequest {
        order: TriggerOrder::Name.into(),
        page: Some(PageRequest {
            size: 2,
            token: String::new(),
        }),
        ..request(&place)
    };
    let crons = ListWorkspaceTriggersRequest {
        kinds: vec![TriggerKind::Cron.into()],
        ..request(&place)
    };

    // when
    let all = app
        .list_workspace_triggers(place.owner, request(&place))
        .await
        .unwrap();
    let first = app
        .list_workspace_triggers(place.owner, by_name.clone())
        .await
        .unwrap();
    let token = first.page.clone().unwrap().next_token;
    let next = ListWorkspaceTriggersRequest {
        page: Some(PageRequest { size: 2, token }),
        ..by_name
    };
    let second = app
        .list_workspace_triggers(place.owner, next)
        .await
        .unwrap();
    let crons = app
        .list_workspace_triggers(place.owner, crons)
        .await
        .unwrap();

    // then
    assert_eq!(all.time_zone, "Europe/Paris");
    let names = |list: &[scylla_proto::pipelines::v1::TriggerSummary]| -> Vec<String> {
        list.iter()
            .map(|one| one.trigger.clone().unwrap().name)
            .collect()
    };
    assert_eq!(names(&all.triggers).last().unwrap(), "hook");
    let nexts: Vec<_> = all.triggers[..2]
        .iter()
        .map(|one| one.trigger.clone().unwrap().next_fire_at.unwrap().seconds)
        .collect();
    assert!(nexts[0] <= nexts[1]);
    for summary in &all.triggers {
        let trigger = summary.trigger.clone().unwrap();
        let count = summary.upcoming_fires.len();
        match trigger.name.as_str() {
            "hourly" => assert!((23..=24).contains(&count), "{count}"),
            "nightly" => assert!(count <= 1, "{count}"),
            _ => assert_eq!(count, 0),
        }
        if let Some(first) = summary.upcoming_fires.first() {
            assert_eq!(Some(first), trigger.next_fire_at.as_ref());
        }
    }
    assert_eq!(names(&first.triggers), vec!["hook", "hourly"]);
    assert_eq!(names(&second.triggers), vec!["nightly"]);
    assert_eq!(names(&crons.triggers).len(), 2);
}

/// Each summary gives its last fire and its failed fires and missed times since the start of the
/// period. A viewer of one project sees its triggers only, and a user out of the workspace sees
/// none.
#[sqlx::test(migrations = false)]
async fn list_workspace_triggers_gives_the_results_that_the_caller_sees(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let nightly = trigger(&app, &place, 0, "nightly", Some("0 3 * * *")).await;
    trigger(&app, &place, 2, "hourly", Some("0 * * * *")).await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let nightly_id = text(&nightly.id).to_owned();
    for (due, status) in [
        (now - 30 * HOUR, "error"),
        (now - 3 * HOUR, "error"),
        (now - 2 * HOUR, "missed"),
    ] {
        fire(&app, &nightly_id, due, status).await;
    }
    let (viewer, stranger) = (app.user().await, app.user().await);
    app.join_workspace(place.owner, viewer).await;
    let role = Role::on("project", &place.projects[1], "viewer");
    app.grant_role(place.owner, role, &viewer.to_string())
        .await
        .unwrap();

    // when
    let owned = app
        .list_workspace_triggers(place.owner, request(&place))
        .await
        .unwrap();
    let seen = app
        .list_workspace_triggers(viewer, request(&place))
        .await
        .unwrap();
    let none = app
        .list_workspace_triggers(stranger, request(&place))
        .await
        .unwrap();

    // then
    let summary = owned
        .triggers
        .iter()
        .find(|one| one.trigger.as_ref().unwrap().name == "nightly")
        .unwrap();
    assert_eq!((summary.failed_fires, summary.missed_times), (1, 1));
    let last = summary.last_fire.as_ref().unwrap();
    assert_eq!(last.status(), FireStatus::Missed);
    assert!(!summary.pipeline_name.is_empty());
    assert_eq!(owned.triggers.len(), 2);
    assert_eq!(seen.triggers.len(), 1);
    assert_eq!(seen.triggers[0].trigger.as_ref().unwrap().name, "hourly");
    assert!(none.triggers.is_empty());
}

/// More than 100 project ids, an unset kind, an unknown order, or a window above 7 days gives
/// INVALID_ARGUMENT on its field.
#[sqlx::test(migrations = false)]
async fn list_workspace_triggers_refuses_a_bad_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let good = request(&place);
    let week = Duration {
        seconds: 7 * 24 * HOUR + 1,
        nanos: 0,
    };
    let cases = [
        (
            "project_ids",
            ListWorkspaceTriggersRequest {
                project_ids: vec![id(place.projects[0].as_str()); 101],
                ..good.clone()
            },
        ),
        (
            "kinds",
            ListWorkspaceTriggersRequest {
                kinds: vec![0],
                ..good.clone()
            },
        ),
        (
            "order",
            ListWorkspaceTriggersRequest {
                order: 9,
                ..good.clone()
            },
        ),
        (
            "upcoming",
            ListWorkspaceTriggersRequest {
                upcoming: Some(week),
                ..good.clone()
            },
        ),
    ];

    for (field, request) in cases {
        // when
        let status = app
            .list_workspace_triggers(place.owner, request)
            .await
            .expect_err(field);

        // then
        assert_invalid_field(&status, field);
    }
}
