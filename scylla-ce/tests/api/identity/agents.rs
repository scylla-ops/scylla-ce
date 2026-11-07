//! The tests of the stream of an agent: the hello, the presence, the renewal of the token, and
//! each end of a stream.

use std::time::Duration;

use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::apps::AppKeyId;
use scylla_domain::identity::apps::KIND;
use scylla_extension::Binding;
use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::agents::v1::{Hello, OpenStreamRequest, OpenStreamResponse, Renew};
use scylla_proto::apps::v1::Host;
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tokio::time::timeout;
use tonic::{Code, Status, Streaming};
use tonic_types::StatusExt;

use crate::{app, assert_code};

/// The longest wait for a message of the server in a test.
const WAIT: Duration = Duration::from_secs(10);

/// The host of the test agent.
fn host() -> Host {
    Host {
        version: "1.0.0".to_owned(),
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
        hostname: " runner-1 ".to_owned(),
        cpu_count: 8,
        memory_bytes: 16 << 30,
        capacity: 0,
        container_runtime: "docker".to_owned(),
    }
}

/// Makes a hello message.
fn hello() -> OpenStreamRequest {
    OpenStreamRequest {
        message: Some(Up::Hello(Hello {
            host: Some(host()),
            running_jobs: Vec::new(),
            conditional_nodes: true,
        })),
    }
}

/// Makes a renew message.
fn renew(token: &str) -> OpenStreamRequest {
    OpenStreamRequest {
        message: Some(Up::Renew(Renew {
            token: token.to_owned(),
        })),
    }
}

/// Reads the next message of the server.
async fn next(stream: &mut Streaming<OpenStreamResponse>) -> Result<Down, Status> {
    let message = timeout(WAIT, stream.message())
        .await
        .expect("a message in time")?;
    Ok(message.expect("an open stream").message.expect("a message"))
}

/// Opens the stream of a new agent of a user and sends the hello. Returns the app id, the
/// sender, and the stream after the welcome.
async fn connected(
    app: &TestApp,
    user: scylla_domain::identity::actors::ActorId,
) -> (
    scylla_testkit::TestKey,
    tokio::sync::mpsc::Sender<OpenStreamRequest>,
    Streaming<OpenStreamResponse>,
) {
    let workspace = app.workspace_of(user).await;
    let (key, token) = app.app_in(user, workspace, true).await;
    let (sender, mut stream) = app.open_stream(&token).await.unwrap();
    sender.send(hello()).await.unwrap();
    assert!(matches!(next(&mut stream).await, Ok(Down::Welcome(_))));
    (key, sender, stream)
}

/// The hello records the host. The agent is connected while its stream is open, and its last
/// seen time stays after the end.
#[sqlx::test(migrations = false)]
async fn open_stream_records_the_host_and_the_presence(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;

    // when
    let (key, sender, stream) = connected(&app, user).await;
    let open = app.get_app(user, &key.app).await.unwrap().app.unwrap();
    drop((sender, stream));
    let mut closed = app.get_app(user, &key.app).await.unwrap().app.unwrap();
    for _ in 0..50 {
        if !closed.connected {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        closed = app.get_app(user, &key.app).await.unwrap().app.unwrap();
    }

    // then
    assert!(open.connected && !closed.connected);
    let host = open.host.unwrap();
    assert_eq!((host.hostname.as_str(), host.capacity), ("runner-1", 1));
    assert_eq!(host.container_runtime, "docker");
    assert!(closed.last_seen_at.is_some());
}

/// Only an App that runs jobs opens a stream: `NOT_A_RUNNER` for another App and for a user.
#[sqlx::test(migrations = false)]
async fn open_stream_needs_an_agent(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (_, client) = app.app_in(user, workspace, false).await;
    let person = app.token(user).await;

    // when
    let by_client = app.open_stream(&client).await;
    let by_person = app.open_stream(&person).await;

    // then
    for result in [by_client, by_person] {
        let status = result.expect_err("a refusal");
        assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
        let info = status.get_details_error_info().expect("ErrorInfo");
        assert_eq!(info.reason, "NOT_A_RUNNER");
    }
}

/// The first message must be a hello.
#[sqlx::test(migrations = false)]
async fn open_stream_needs_a_hello_first(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (_, token) = app.app_in(user, workspace, true).await;
    let (sender, mut stream) = app.open_stream(&token).await.unwrap();

    // when
    sender.send(renew(&token)).await.unwrap();

    // then
    assert_code(next(&mut stream).await, Code::InvalidArgument);
}

/// A Renew with a new token of the agent keeps the stream. A token of another actor ends it.
#[sqlx::test(migrations = false)]
async fn renew_needs_a_token_of_the_same_agent(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let (key, sender, mut stream) = connected(&app, user).await;
    let fresh = app.exchange(key.assertion(60)).await.unwrap();
    let other = app.token(user).await;

    // when
    sender.send(renew(&fresh)).await.unwrap();
    let renewed = next(&mut stream).await;
    sender.send(renew(&other)).await.unwrap();
    let refused = next(&mut stream).await;

    // then
    assert!(matches!(renewed, Ok(Down::Renewed(_))));
    assert_code(refused, Code::Unauthenticated);
}

/// With no Renew, the stream ends at the expiry of its token.
#[sqlx::test(migrations = false)]
async fn the_stream_ends_at_the_expiry_of_its_token(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (key, _) = app.app_in(user, workspace, true).await;
    let actor = Actor {
        id: key.actor(),
        kind: KIND,
    };
    let binding = Binding::Key(AppKeyId::parse(&key.key_id).unwrap());
    let short = app
        .access
        .issue_for(&actor, Duration::from_secs(2), binding)
        .unwrap();
    let (sender, mut stream) = app.open_stream(&short.value).await.unwrap();
    sender.send(hello()).await.unwrap();
    let welcome = next(&mut stream).await;

    // when
    let end = next(&mut stream).await;

    // then
    assert!(matches!(welcome, Ok(Down::Welcome(_))));
    assert_code(end, Code::Unauthenticated);
}

/// A second stream of the agent, a revoked key and a deleted App end a stream with `ABORTED`.
#[sqlx::test(migrations = false)]
async fn the_server_stops_a_stream(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let (key, _first_sender, mut first) = connected(&app, user).await;
    let token = app.exchange(key.assertion(60)).await.unwrap();
    let (second_sender, mut second) = app.open_stream(&token).await.unwrap();
    second_sender.send(hello()).await.unwrap();
    assert!(matches!(next(&mut second).await, Ok(Down::Welcome(_))));
    let (other, _other_sender, mut third) = connected(&app, user).await;

    // when
    let replaced = next(&mut first).await;
    app.revoke_app_key(user, &key.app, &key.key_id)
        .await
        .unwrap();
    let revoked = next(&mut second).await;
    app.delete_app(user, &other.app).await.unwrap();
    let deleted = next(&mut third).await;

    // then
    assert_code(replaced, Code::Aborted);
    assert_code(revoked, Code::Aborted);
    assert_code(deleted, Code::Aborted);
}
