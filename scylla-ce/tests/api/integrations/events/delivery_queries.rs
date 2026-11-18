//! The tests of the queries of `GetDeliveryStats`: the hours of the time zone of the workspace,
//! who reads the counts, and the bad requests.

use scylla_proto::integrations::v1::{DeliveryStatsBucket, GetDeliveryStatsRequest};
use scylla_testkit::{Role, RoleInput};
use serde_json::json;
use sqlx::PgPool;
use tonic::Code;

use super::delivery_stats::{
    by_id, counts, fire, move_delivery, move_fires, now, place, stats, HOUR,
};
use super::{app_with_fake, github_push, gitlab_push, post_app, post_gitlab, SHA, SIGNING_TOKEN};
use crate::assert_invalid_field;

/// The buckets are the hours of the time zone of the workspace, also the hours with no delivery,
/// and a delivery out of the period is in no count.
#[sqlx::test(migrations = false)]
async fn get_delivery_stats_puts_each_delivery_in_an_hour_of_the_workspace(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let place = place(&app).await;
    app.set_workspace_time_zone(place.owner, &place.workspace, "Asia/Kolkata")
        .await
        .unwrap();
    let base = (now() / HOUR - 24) * HOUR + HOUR / 2;
    let pushes = [
        ("gh-1", "main", 20),
        ("gh-2", "dev", 70),
        ("gh-3", "fix", 10),
    ];
    for (key, branch, _) in pushes {
        post_app(&app, "push", key, &github_push(SHA, branch)).await;
        app.wait_for_delivery(key).await;
    }
    post_gitlab(
        &app,
        &place.gitlab_project,
        "gl-1",
        &gitlab_push(SHA, "main"),
        SIGNING_TOKEN,
        0,
    )
    .await;
    app.wait_for_delivery("gl-1").await;
    for (key, _, minutes) in pushes {
        move_delivery(&app, key, base + minutes * 60).await;
    }
    move_delivery(&app, "gl-1", base + 140 * 60).await;
    fire(&app, &place, "hook-1", json!({ "after": SHA })).await;
    move_fires(&app, &place.hook, base + 125 * 60).await;

    // when
    let request = GetDeliveryStatsRequest {
        bucket: DeliveryStatsBucket::Hour.into(),
        ..stats(&place.workspace, base + 15 * 60, Some(base + 135 * 60))
    };
    let found = app.get_delivery_stats(place.owner, request).await.unwrap();

    // then
    let buckets: Vec<_> = found
        .buckets
        .iter()
        .map(|one| (one.start.unwrap().seconds - base, counts(&one.figures)))
        .collect();
    assert_eq!(buckets, [(0, (1, 0)), (HOUR, (1, 0)), (2 * HOUR, (1, 0))]);
    assert_eq!(counts(&found.totals), (3, 0));
    assert_eq!(found.connections.len(), 1);
    assert_eq!(found.time_zone, "Asia/Kolkata");
}

/// A role with the view of the integrations reads the connections, and the webhook triggers of
/// the projects where it can count the fires only. A user with no role on the workspace and a
/// viewer of a project with no view of the integrations are refused.
#[sqlx::test(migrations = false)]
async fn get_delivery_stats_shows_the_triggers_that_the_caller_can_count(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let place = place(&app).await;
    post_app(&app, "push", "gh-1", &github_push(SHA, "main")).await;
    app.wait_for_delivery("gh-1").await;
    fire(&app, &place, "hook-1", json!({ "after": SHA })).await;
    let input = RoleInput {
        name: "Integrations",
        level: "workspace",
        holder: "people",
        permissions: &["integrations.view"],
        workspace: Some(&place.workspace),
    };
    let role = app.create_role(place.owner, input).await.unwrap();
    let [reader, project_viewer, stranger] = [app.user().await, app.user().await, app.user().await];
    let grant = Role::on("workspace", &place.workspace, &role.grant_name);
    app.grant_role(place.owner, grant, &reader.to_string())
        .await
        .unwrap();
    app.join_workspace(place.owner, project_viewer).await;
    let viewer = Role::on("project", &place.hook_project, "viewer");
    app.grant_role(place.owner, viewer, &project_viewer.to_string())
        .await
        .unwrap();
    let request = || stats(&place.workspace, now() - HOUR, None);

    // when
    let before = app.get_delivery_stats(reader, request()).await.unwrap();
    let viewer = Role::on("project", &place.hook_project, "viewer");
    app.grant_role(place.owner, viewer, &reader.to_string())
        .await
        .unwrap();
    let after = app.get_delivery_stats(reader, request()).await.unwrap();
    let refused = app.get_delivery_stats(project_viewer, request()).await;
    let none = app.get_delivery_stats(stranger, request()).await;

    // then
    assert_eq!(by_id(&before), [(place.github.clone(), (1, 0))]);
    assert_eq!(counts(&before.totals), (1, 0));
    let expected = [(place.github.clone(), (1, 0)), (place.hook.clone(), (1, 0))];
    assert_eq!(by_id(&after), expected);
    assert_eq!(counts(&after.totals), (2, 0));
    assert_eq!(refused.unwrap_err().code(), Code::PermissionDenied);
    assert_eq!(none.unwrap_err().code(), Code::PermissionDenied);
}

/// No start, an end that is not after the start, a period of more than 400 days and more than
/// 1000 hours are refused.
#[sqlx::test(migrations = false)]
async fn get_delivery_stats_refuses_a_bad_period(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let place = place(&app).await;
    let start = now();
    let no_start = GetDeliveryStatsRequest {
        from: None,
        ..stats(&place.workspace, start, None)
    };
    let hours = GetDeliveryStatsRequest {
        bucket: DeliveryStatsBucket::Hour.into(),
        ..stats(&place.workspace, start - 50 * 24 * HOUR, Some(start))
    };

    // when
    let missing = app.get_delivery_stats(place.owner, no_start).await;
    let request = stats(&place.workspace, start, Some(start - HOUR));
    let reversed = app.get_delivery_stats(place.owner, request).await;
    let request = stats(&place.workspace, start - 401 * 24 * HOUR, Some(start));
    let long = app.get_delivery_stats(place.owner, request).await;
    let many = app.get_delivery_stats(place.owner, hours).await;

    // then
    assert_invalid_field(&missing.unwrap_err(), "from");
    assert_invalid_field(&reversed.unwrap_err(), "to");
    assert_invalid_field(&long.unwrap_err(), "to");
    assert_invalid_field(&many.unwrap_err(), "bucket");
}
