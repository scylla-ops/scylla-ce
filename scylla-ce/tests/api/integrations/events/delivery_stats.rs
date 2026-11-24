//! The tests of `GetDeliveryStats`: the deliveries of each connection and of each webhook
//! trigger, and their newest delivery. The helpers of the place of a test are here too.

use std::time::{SystemTime, UNIX_EPOCH};

use prost_types::Timestamp;
use scylla_domain::identity::actors::ActorId;
use scylla_proto::integrations::v1::{
    DeliveryFigures, GetDeliveryStatsRequest, GetDeliveryStatsResponse,
};
use scylla_proto::pipelines::v1::trigger;
use scylla_testkit::{exec_node, id, pointer_input, text, LinkRow, TestApp};
use serde_json::json;
use sqlx::PgPool;

use super::{
    app_with_fake, github_push, gitlab_push, post_app, post_github, post_gitlab, GITLAB_TOKEN, SHA,
    SIGNING_TOKEN,
};

/// One hour, in seconds.
pub(super) const HOUR: i64 = 3_600;

/// A workspace with a GitHub connection (installation 42, `octo/app` linked), a GitLab
/// connection (`group/app` linked with a webhook) and a webhook trigger in a third project.
pub(super) struct Place {
    /// The owner of the workspace.
    pub(super) owner: ActorId,
    /// The workspace.
    pub(super) workspace: String,
    /// The GitHub connection.
    pub(super) github: String,
    /// The GitLab connection.
    pub(super) gitlab: String,
    /// The project linked to GitLab.
    pub(super) gitlab_project: String,
    /// The project of the webhook trigger.
    pub(super) hook_project: String,
    /// The webhook trigger.
    pub(super) hook: String,
    /// The route of the webhook trigger.
    pub(super) hook_path: String,
    /// The secret of the webhook trigger.
    pub(super) hook_secret: String,
}

/// Makes the place of a test. The webhook trigger reads `/after` of the body.
pub(super) async fn place(app: &TestApp) -> Place {
    // Link a project to GitHub and a project to GitLab.
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let mut projects = Vec::new();
    for name in ["API", "Web", "Ops"] {
        let project = app.create_project(owner, name).await.unwrap();
        projects.push(text(&project.id).to_owned());
    }
    let github = app
        .insert_github_connection(workspace, owner, 42, "octo")
        .await;
    let gitlab = app
        .insert_gitlab_connection(workspace, owner, "scylla-bot", GITLAB_TOKEN)
        .await;
    for (project, connection, full_name) in [
        (&projects[0], &github, "octo/app"),
        (&projects[1], &gitlab, "group/app"),
    ] {
        let link = LinkRow {
            project,
            connection,
            external_id: "7",
            full_name,
            default_branch: "main",
        };
        app.insert_repository_link(&link).await;
    }
    app.set_link_signing_token(&projects[1], "9", SIGNING_TOKEN)
        .await;

    // Add a webhook trigger to the third project.
    let pipeline = app
        .create_pipeline(owner, &projects[2], vec![exec_node("build", &[])])
        .await
        .unwrap();
    let inputs = vec![pointer_input("SHA", "/after")];
    let created = app
        .create_trigger(owner, text(&pipeline.id), "deploy-hook", None, inputs)
        .await
        .unwrap();
    let hook = created.trigger.unwrap();
    let Some(trigger::Kind::Webhook(webhook)) = hook.kind else {
        panic!("a webhook trigger")
    };
    Place {
        owner,
        workspace: workspace.to_string(),
        github,
        gitlab,
        gitlab_project: projects[1].clone(),
        hook_project: projects[2].clone(),
        hook: text(&hook.id).to_owned(),
        hook_path: webhook.path,
        hook_secret: created.webhook_secret,
    }
}

/// Returns the request of the counts of a workspace from `from` to `to`, in seconds since 1970.
pub(super) fn stats(workspace: &str, from: i64, to: Option<i64>) -> GetDeliveryStatsRequest {
    let at = |seconds| Timestamp { seconds, nanos: 0 };
    GetDeliveryStatsRequest {
        workspace_id: Some(id(workspace)),
        from: Some(at(from)),
        to: to.map(at),
        ..GetDeliveryStatsRequest::default()
    }
}

/// Returns now, in seconds since 1970.
pub(super) fn now() -> i64 {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    i64::try_from(elapsed.as_secs()).unwrap()
}

/// Returns the delivered and the failed deliveries of counts.
pub(super) fn counts(figures: &Option<DeliveryFigures>) -> (u32, u32) {
    let figures = figures.as_ref().expect("figures");
    (figures.delivered, figures.failed)
}

/// Sends a signed delivery to the webhook trigger. Returns the status.
pub(super) async fn fire(app: &TestApp, place: &Place, key: &str, body: serde_json::Value) -> u16 {
    let (path, secret) = (&place.hook_path, &place.hook_secret);
    post_github(app, path, "push", key, &body, secret).await.0
}

/// Moves the arrival of a delivery of a connection to `at` seconds since 1970.
pub(super) async fn move_delivery(app: &TestApp, key: &str, at: i64) {
    sqlx::query("UPDATE vcs_deliveries SET received_at = to_timestamp($2) WHERE delivery_id = $1")
        .bind(key)
        .bind(at as f64)
        .execute(&app.pool)
        .await
        .unwrap();
}

/// Moves the arrival of each delivery of a webhook trigger to `at` seconds since 1970.
pub(super) async fn move_fires(app: &TestApp, trigger: &str, at: i64) {
    sqlx::query(
        "UPDATE trigger_fires SET due_at = to_timestamp($2), fired_at = to_timestamp($2)
         WHERE trigger_id = $1::uuid",
    )
    .bind(trigger)
    .bind(at as f64)
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Returns the counts of the connections and of the triggers of an answer, by id.
pub(super) fn by_id(stats: &GetDeliveryStatsResponse) -> Vec<(String, (u32, u32))> {
    let connections = stats
        .connections
        .iter()
        .map(|one| (text(&one.connection_id).to_owned(), counts(&one.figures)));
    let triggers = stats
        .triggers
        .iter()
        .map(|one| (text(&one.trigger_id).to_owned(), counts(&one.figures)));
    connections.chain(triggers).collect()
}

/// Each connection counts the signed deliveries of its installation or of its links, with the
/// failed ones and its newest delivery. The webhook trigger counts its signed deliveries, and a
/// delivery that made no job fails. A bad signature and another installation count nothing.
#[sqlx::test(migrations = false)]
async fn get_delivery_stats_counts_each_connection_and_each_webhook_trigger(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let place = place(&app).await;
    let mut elsewhere = github_push(SHA, "main");
    elsewhere["installation"]["id"] = json!(99);
    let mut statuses = vec![
        post_app(&app, "push", "gh-1", &github_push(SHA, "main"))
            .await
            .0,
        post_app(&app, "push", "gh-2", &github_push(SHA, "dev"))
            .await
            .0,
        post_app(&app, "push", "gh-3", &elsewhere).await.0,
        post_gitlab(
            &app,
            &place.gitlab_project,
            "gl-1",
            &gitlab_push(SHA, "main"),
            SIGNING_TOKEN,
            0,
        )
        .await
        .0,
        post_github(
            &app,
            "/integrations/github/webhook",
            "push",
            "gh-4",
            &json!({}),
            "bad",
        )
        .await
        .0,
    ];
    statuses.push(fire(&app, &place, "hook-1", json!({ "after": SHA })).await);
    statuses.push(fire(&app, &place, "hook-2", json!({})).await);
    let forged = post_github(&app, &place.hook_path, "push", "hook-3", &json!({}), "bad").await;
    statuses.push(forged.0);
    for key in ["gh-1", "gh-2", "gh-3", "gl-1"] {
        app.wait_for_delivery(key).await;
    }
    let start = now();
    move_delivery(&app, "gh-1", start - 20).await;
    move_delivery(&app, "gh-2", start - 10).await;
    sqlx::query("UPDATE vcs_deliveries SET error = 'the work failed' WHERE delivery_id = 'gh-1'")
        .execute(&app.pool)
        .await
        .unwrap();

    // when
    let request = stats(&place.workspace, start - HOUR, None);
    let found = app.get_delivery_stats(place.owner, request).await.unwrap();

    // then
    assert_eq!(statuses, [202, 202, 202, 202, 401, 202, 422, 401]);
    assert_eq!(counts(&found.totals), (5, 2));
    let mut seen = by_id(&found);
    seen.sort();
    let mut expected = vec![
        (place.github.clone(), (2, 1)),
        (place.gitlab.clone(), (1, 0)),
        (place.hook.clone(), (2, 1)),
    ];
    expected.sort();
    assert_eq!(seen, expected);
    let github = found
        .connections
        .iter()
        .find(|one| text(&one.connection_id) == place.github)
        .unwrap();
    let last = (github.last_event.as_str(), github.last_repository.as_str());
    assert_eq!(last, ("push", "octo/app"));
    assert_eq!(github.last_ref, "refs/heads/dev");
    assert_eq!(github.last_received_at.unwrap().seconds, start - 10);
    let gitlab = found
        .connections
        .iter()
        .find(|one| text(&one.connection_id) == place.gitlab)
        .unwrap();
    let last = (gitlab.last_repository.as_str(), gitlab.last_ref.as_str());
    assert_eq!(last, ("group/app", "refs/heads/main"));
    assert!(found.triggers[0].last_received_at.is_some());
    assert!(found.buckets.is_empty());
    assert_eq!(found.time_zone, "UTC");
}
