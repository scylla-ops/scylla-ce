//! The tests of the open items of the dashboard that depend on the caller and on the agents: the
//! kinds that each role reads, the jobs that wait for an approval, the filter of projects, the
//! agents that are not connected, and the bad requests.

use std::time::Duration;

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::JobId;
use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::agents::v1::{Hello, OpenStreamRequest};
use scylla_proto::apps::v1::Host;
use scylla_proto::common::v1::PageRequest;
use scylla_proto::dashboards::v1::AttentionKind::{
    AgentOffline, ApprovalWaiting, FireFailed, FireMissed, JobFailed, JobOrphaned, Unspecified,
};
use scylla_proto::dashboards::v1::ListAttentionItemsRequest;
use scylla_testkit::{id, text, Role, TestApp};
use sqlx::PgPool;
use tokio::time::timeout;

use super::attention::{counts, cron, ended, fire, items, request};
use super::job_stats::{place, Place};
use crate::{app, assert_invalid_field};

/// Starts a job of a pipeline, then makes it wait for an approval since `ago` minutes. Returns
/// the id of the job.
async fn waiting(app: &TestApp, place: &Place, pipeline: usize, ago: i32) -> String {
    let job = app.run_pipeline(place.owner, &place.pipelines[pipeline]);
    let job = text(&job.await.unwrap().id).to_owned();
    sqlx::query!(
        "UPDATE jobs SET status = 'awaiting_approval', approval = 'waiting',
                created_at = now() - make_interval(mins => $2::int4)
         WHERE id = $1",
        JobId::parse(&job).unwrap().to_db(),
        ago,
    )
    .execute(&app.pool)
    .await
    .unwrap();
    job
}

/// Sets the last time when the server saw an App to `ago` minutes ago.
async fn seen(app: &TestApp, agent: &str, ago: i32) {
    sqlx::query!(
        "UPDATE apps SET last_seen_at = now() - make_interval(mins => $2::int4) WHERE id = $1",
        AppId::parse(agent).unwrap().to_db(),
        ago,
    )
    .execute(&app.pool)
    .await
    .unwrap();
}

/// Creates an App of the workspace of a place, and returns its id.
async fn agent(app: &TestApp, place: &Place, runner: bool) -> String {
    let created = app.create_app(place.owner, place.workspace, runner).await;
    text(&created.unwrap().app.unwrap().id).to_owned()
}

/// Makes a hello message.
fn hello() -> OpenStreamRequest {
    let host = Host {
        version: "1.0.0".to_owned(),
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        hostname: "runner-1".to_owned(),
        cpu_count: 8,
        memory_bytes: 16 << 30,
        capacity: 0,
        container_runtime: "docker".to_owned(),
        features: Vec::new(),
        protocol: 2,
        build: String::new(),
        started_at: None,
    };
    let hello = Hello {
        host: Some(host),
        running_jobs: Vec::new(),
    };
    OpenStreamRequest {
        message: Some(Up::Hello(hello)),
    }
}

/// Each caller gets the kinds that it reads, in the projects where it reads them: a viewer of
/// the workspace gets no approval, an editor of a project gets its jobs and its approvals but no
/// agent, and a user out of the workspace gets nothing. A filter of projects gives no agent.
#[sqlx::test(migrations = false)]
async fn the_list_drops_the_kinds_that_the_caller_cannot_read(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let workspace = place.workspace.to_string();
    ended(&app, &place, 0, "failed", 30, None).await;
    ended(&app, &place, 2, "failed", 20, None).await;
    let approval = waiting(&app, &place, 0, 10).await;
    let nightly = cron(&app, &place, 2, "nightly").await;
    fire(&app, &nightly, 15, "error").await;
    seen(&app, &agent(&app, &place, true).await, 12).await;
    let (watcher, editor, viewer) = (app.user().await, app.user().await, app.user().await);
    let stranger = app.user().await;
    let roles = [
        (watcher, Role::on("workspace", &workspace, "viewer")),
        (editor, Role::on("project", &place.projects[0], "editor")),
        (viewer, Role::on("project", &place.projects[1], "viewer")),
    ];
    for (user, role) in roles {
        if user != watcher {
            app.join_workspace(place.owner, user).await;
        }
        app.grant_role(place.owner, role, &user.to_string())
            .await
            .unwrap();
    }
    let beta = ListAttentionItemsRequest {
        project_ids: vec![id(place.projects[1].as_str())],
        ..request(place.workspace)
    };

    // when
    let all = request(place.workspace);
    let owned = app.list_attention_items(place.owner, all.clone()).await;
    let watched = app.list_attention_items(watcher, all.clone()).await;
    let edited = app.list_attention_items(editor, all.clone()).await;
    let viewed = app.list_attention_items(viewer, all.clone()).await;
    let none = app.list_attention_items(stranger, all).await.unwrap();
    let filtered = app.list_attention_items(place.owner, beta).await.unwrap();

    // then
    let owned = owned.unwrap();
    let expected = vec![
        (JobFailed, 2),
        (JobOrphaned, 0),
        (FireFailed, 1),
        (FireMissed, 0),
        (AgentOffline, 1),
        (ApprovalWaiting, 1),
    ];
    assert_eq!(counts(&owned), expected);
    assert_eq!(owned.open_count, 5);
    assert_eq!(items(&owned)[0], (ApprovalWaiting, 1, approval));
    let expected = vec![
        (JobFailed, 2),
        (JobOrphaned, 0),
        (FireFailed, 1),
        (FireMissed, 0),
        (AgentOffline, 1),
    ];
    assert_eq!(counts(&watched.unwrap()), expected);
    let expected = vec![
        (JobFailed, 1),
        (JobOrphaned, 0),
        (FireFailed, 0),
        (FireMissed, 0),
        (ApprovalWaiting, 1),
    ];
    assert_eq!(counts(&edited.unwrap()), expected);
    let expected = vec![
        (JobFailed, 1),
        (JobOrphaned, 0),
        (FireFailed, 1),
        (FireMissed, 0),
    ];
    assert_eq!(counts(&viewed.unwrap()), expected);
    assert!(none.counts.is_empty() && none.items.is_empty());
    assert_eq!(none.open_count, 0);
    let expected = vec![
        (JobFailed, 1),
        (JobOrphaned, 0),
        (FireFailed, 1),
        (FireMissed, 0),
        (ApprovalWaiting, 0),
    ];
    assert_eq!(counts(&filtered), expected);
}

/// An enabled agent that the server last saw more than 5 minutes ago is open while it is not
/// connected. An agent seen 2 minutes ago, an agent that never connected, an App that runs no
/// job and a disabled agent are not open.
#[sqlx::test(migrations = false)]
async fn an_agent_is_open_while_it_is_not_connected(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let gone = agent(&app, &place, true).await;
    seen(&app, &gone, 10).await;
    seen(&app, &agent(&app, &place, true).await, 2).await;
    agent(&app, &place, true).await;
    seen(&app, &agent(&app, &place, false).await, 10).await;
    let disabled = agent(&app, &place, true).await;
    app.disable_app(place.owner, &disabled).await.unwrap();
    seen(&app, &disabled, 10).await;
    let (key, token) = app.app_in(place.owner, place.workspace, true).await;
    let (sender, mut stream) = app.open_stream(&token).await.unwrap();
    sender.send(hello()).await.unwrap();
    let welcome = timeout(Duration::from_secs(10), stream.message()).await;
    let message = welcome.unwrap().unwrap().unwrap().message;
    assert!(matches!(message, Some(Down::Welcome(_))));
    seen(&app, &key.app, 10).await;
    let agents = ListAttentionItemsRequest {
        kinds: vec![AgentOffline.into()],
        ..request(place.workspace)
    };

    // when
    let connected = app.list_attention_items(place.owner, agents.clone()).await;
    drop((sender, stream));
    let mut state = app
        .get_app(place.owner, &key.app)
        .await
        .unwrap()
        .app
        .unwrap();
    for _ in 0..50 {
        if !state.connected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        state = app
            .get_app(place.owner, &key.app)
            .await
            .unwrap()
            .app
            .unwrap();
    }
    seen(&app, &key.app, 10).await;
    let closed = app.list_attention_items(place.owner, agents).await.unwrap();

    // then
    let connected = connected.unwrap();
    assert_eq!(items(&connected), vec![(AgentOffline, 1, gone.clone())]);
    assert_eq!(counts(&connected), vec![(AgentOffline, 1)]);
    let mut found: Vec<String> = items(&closed).into_iter().map(|(_, _, id)| id).collect();
    found.sort();
    let mut expected = vec![gone, key.app];
    expected.sort();
    assert_eq!(found, expected);
    assert!(closed.items[0].project_id.is_none());
}

/// A request with no workspace, more than 100 projects, an unset kind or a bad page token gives
/// INVALID_ARGUMENT on its field.
#[sqlx::test(migrations = false)]
async fn list_attention_items_refuses_a_bad_request(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = place(&app).await;
    let good = request(place.workspace);
    let projects = (0..101).map(|_| id(place.projects[0].as_str())).collect();
    let cases = [
        ("workspace_id", ListAttentionItemsRequest::default()),
        (
            "project_ids",
            ListAttentionItemsRequest {
                project_ids: projects,
                ..good.clone()
            },
        ),
        (
            "kinds",
            ListAttentionItemsRequest {
                kinds: vec![Unspecified.into()],
                ..good.clone()
            },
        ),
        (
            "page.token",
            ListAttentionItemsRequest {
                page: Some(PageRequest {
                    size: 10,
                    token: "a.b".to_owned(),
                }),
                ..good.clone()
            },
        ),
    ];

    for (field, request) in cases {
        // when
        let status = app
            .list_attention_items(place.owner, request)
            .await
            .expect_err(field);

        // then
        assert_invalid_field(&status, field);
    }
}
