//! The tests of the samples that an agent sends on its stream: the interval in the welcome, the
//! last sample of each App, the samples that the server ignores, and the start of the agent.

use prost_types::Timestamp;
use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::agents::v1::{Hello, Metrics, OpenStreamRequest};
use scylla_proto::apps::v1::Host;
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tokio::sync::mpsc::Sender;
use tonic::Streaming;

use super::agents::{connected, host, next, renew};
use crate::app;

/// Makes a message with a sample.
fn metrics(cpu_percent: f32, memory_used_bytes: u64) -> OpenStreamRequest {
    OpenStreamRequest {
        message: Some(Up::Metrics(Metrics {
            cpu_percent,
            memory_used_bytes,
        })),
    }
}

/// Sends a Renew with a new token, and waits for the answer: the server read each message before
/// it.
async fn sync(
    app: &TestApp,
    key: &scylla_testkit::TestKey,
    sender: &Sender<OpenStreamRequest>,
    stream: &mut Streaming<scylla_proto::agents::v1::OpenStreamResponse>,
) {
    let token = app.exchange(key.assertion(60)).await.unwrap();
    sender.send(renew(&token)).await.unwrap();
    assert!(matches!(next(stream).await, Ok(Down::Renewed(_))));
}

/// The welcome asks for a sample each 60 seconds.
#[sqlx::test(migrations = false)]
async fn the_welcome_gives_the_interval_of_the_samples(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (_, token) = app.app_in(user, workspace, true).await;
    let (sender, mut stream) = app.open_stream(&token).await.unwrap();
    let hello = OpenStreamRequest {
        message: Some(Up::Hello(Hello {
            host: Some(host()),
            running_jobs: Vec::new(),
            conditional_nodes: true,
        })),
    };

    // when
    sender.send(hello).await.unwrap();
    let welcome = next(&mut stream).await;

    // then
    let Ok(Down::Welcome(welcome)) = welcome else {
        panic!("a welcome: {welcome:?}");
    };
    assert_eq!(welcome.metrics_interval_seconds, 60);
}

/// The server keeps a sample, and ignores a sample less than 30 seconds after it and a CPU use
/// that is not a number. The stream stays open, and the Apps show the sample that it kept.
#[sqlx::test(migrations = false)]
async fn the_stream_records_the_samples_of_the_agent(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (key, sender, mut stream) = connected(&app, user).await;

    // when
    sender.send(metrics(25.5, 1 << 30)).await.unwrap();
    sender.send(metrics(90.0, 2 << 30)).await.unwrap();
    sender.send(metrics(f32::NAN, 3 << 30)).await.unwrap();
    sync(&app, &key, &sender, &mut stream).await;

    // then
    let got = app.get_app(user, &key.app).await.unwrap().app.unwrap();
    let sample = got.last_sample.expect("a sample");
    assert_eq!(
        (sample.cpu_percent, sample.memory_used_bytes),
        (25.5, 1 << 30)
    );
    assert!(sample.at.is_some());
    let listed = app.list_apps(user, workspace, None, 10, "").await.unwrap();
    assert_eq!(listed.apps[0].last_sample, Some(sample));
}

/// An agent that sends no sample keeps its stream, and its App shows no sample.
#[sqlx::test(migrations = false)]
async fn an_agent_with_no_sample_keeps_its_stream(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let (key, sender, mut stream) = connected(&app, user).await;

    // when
    sync(&app, &key, &sender, &mut stream).await;

    // then
    let got = app.get_app(user, &key.app).await.unwrap().app.unwrap();
    assert!(got.connected);
    assert_eq!(got.last_sample, None);
}

/// The hello records the start of the agent. A start in the future becomes the time of the
/// server.
#[sqlx::test(migrations = false)]
async fn the_hello_records_the_start_of_the_agent(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let started = Timestamp {
        seconds: 1_793_577_600,
        nanos: 0,
    };
    let future = Timestamp {
        seconds: 4_102_444_800,
        nanos: 0,
    };
    let mut keys = Vec::new();
    let mut streams = Vec::new();

    // when
    for start in [started, future] {
        let (key, token) = app.app_in(user, workspace, true).await;
        let (sender, mut stream) = app.open_stream(&token).await.unwrap();
        let hello = Hello {
            host: Some(Host {
                started_at: Some(start),
                ..host()
            }),
            running_jobs: Vec::new(),
            conditional_nodes: true,
        };
        let hello = OpenStreamRequest {
            message: Some(Up::Hello(hello)),
        };
        sender.send(hello).await.unwrap();
        assert!(matches!(next(&mut stream).await, Ok(Down::Welcome(_))));
        keys.push(key);
        streams.push((sender, stream));
    }

    // then
    let mut starts = Vec::new();
    for key in &keys {
        let got = app.get_app(user, &key.app).await.unwrap().app.unwrap();
        starts.push(got.host.unwrap().started_at.expect("a start"));
    }
    assert_eq!(starts[0], started);
    assert!(starts[1].seconds < future.seconds);
}
