//! The tests of the agent protocols on the stream of an agent: the range of the server in the
//! welcome, and the protocol, the build and the compatibility of each agent.

use scylla_proto::agents::v1::open_stream_request::Message as Up;
use scylla_proto::agents::v1::open_stream_response::Message as Down;
use scylla_proto::agents::v1::{Hello, OpenStreamRequest};
use scylla_proto::apps::v1::AgentCompatibility;
use sqlx::PgPool;

use super::agents::{connected, host, next};
use crate::app;

/// The welcome gives the agent protocols of the server, and the App tells the protocol, the
/// build and the compatibility of its agent. An agent older than the protocol connects as
/// outdated.
#[sqlx::test(migrations = false)]
async fn open_stream_tells_the_protocol_and_the_compatibility(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (old_key, token) = app.app_in(user, workspace, true).await;
    let (old_up, mut old_down) = app.open_stream(&token).await.unwrap();
    let mut old_host = host();
    old_host.protocol = 0;
    let hello = Hello {
        host: Some(old_host),
        running_jobs: Vec::new(),
    };
    let hello = OpenStreamRequest {
        message: Some(Up::Hello(hello)),
    };

    // when
    let (key, _sender, _stream) = connected(&app, user).await;
    old_up.send(hello).await.unwrap();
    let old_welcome = next(&mut old_down).await;
    let current = app.get_app(user, &key.app).await.unwrap().app.unwrap();
    let old = app.get_app(user, &old_key.app).await.unwrap().app.unwrap();

    // then
    let Ok(Down::Welcome(welcome)) = old_welcome else {
        panic!("{old_welcome:?}");
    };
    let range = welcome.agent_protocol.expect("the agent protocols");
    assert_eq!((range.min, range.current), (0, 2));
    let host = current.host.expect("a host");
    assert_eq!((host.protocol, host.build.as_str()), (2, "0123abc"));
    assert_eq!(current.compatibility, AgentCompatibility::Compatible as i32);
    assert_eq!(current.missing_features, ["checkout"]);
    assert_eq!(old.compatibility, AgentCompatibility::Outdated as i32);
}
