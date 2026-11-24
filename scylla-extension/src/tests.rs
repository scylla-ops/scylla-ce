//! Unit tests of the contract that the API cannot show: the log event of a refusal and the keys of
//! a notification kind.

use std::io;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use scylla_domain::identity::actors::{Actor, ActorId, ActorKind};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};

use time::OffsetDateTime;

use crate::{
    AccessError, Audience, AudienceContext, Binding, Edition, EventRow, NotificationError,
    NotificationKind, NotificationKinds, Operation, Principal, Recipient,
};

/// An audience with nobody in it.
struct Nobody;

#[async_trait]
impl Audience for Nobody {
    async fn recipients(
        &self,
        _ctx: &AudienceContext<'_>,
        _event: &EventRow,
    ) -> Result<Vec<Recipient>, NotificationError> {
        Ok(Vec::new())
    }
}

/// Makes a kind with this key.
fn kind(key: &'static str) -> NotificationKind {
    NotificationKind::new(key, "builds", Arc::new(Nobody))
}

/// A writer that keeps the log lines in memory.
#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<u8>>>);

impl io::Write for Lines {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().expect("log lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A refusal logs one WARN event `access.refused` with the actor, the route, the permission and
/// the object.
#[test]
fn a_refusal_logs_one_warn_line() {
    // given
    let lines = Lines::default();
    let writer = lines.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let actor = Actor {
        id: ActorId::generate(),
        kind: ActorKind::new("user"),
    };
    let operation = Operation::from_path("/scylla.workspaces.v1.WorkspaceService/GetWorkspace");
    let principal = Principal::new(
        actor.clone(),
        operation,
        Arc::new(()),
        Binding::None,
        OffsetDateTime::now_utc(),
    );
    let workspace = WorkspaceId::generate();
    let check = Workspace::GET_WORKSPACE.on(workspace);

    // when
    let error =
        tracing::subscriber::with_default(subscriber, || AccessError::denied(&principal, &check));

    // then
    let text = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains(" WARN "), "{text}");
    assert!(text.contains("event=\"access.refused\""), "{text}");
    assert!(text.contains(&format!("actor={}", actor.id)), "{text}");
    assert!(text.contains("actor_kind=\"user\""), "{text}");
    assert!(
        text.contains("operation=\"scylla.workspaces.v1.WorkspaceService/GetWorkspace\""),
        "{text}"
    );
    assert!(text.contains("permission=\"get_workspace\""), "{text}");
    assert!(
        text.contains(&format!("object=workspace:{workspace}")),
        "{text}"
    );
    assert!(matches!(error, AccessError::Denied(_)));
}

/// A key must be `<subject>.<action>`, in lower snake case.
#[test]
fn a_key_needs_a_subject_and_an_action() {
    // given
    let mut kinds = NotificationKinds::default();

    // when
    let good = ["deploy.failed", "build_step.timed_out"].map(|key| kinds.add(kind(key)));
    let bad = [
        "deploy",
        "deploy.",
        ".failed",
        "Deploy.failed",
        "a.b.c",
        "a.b-c",
    ]
    .map(|key| kinds.add(kind(key)));

    // then
    assert_eq!(good, [true; 2]);
    assert_eq!(bad, [false; 6]);
    assert_eq!(kinds.all().count(), 2);
}

/// A second kind with the same key stops the start of the edition, and names the key.
#[test]
#[should_panic(expected = "`deploy.failed` cannot be added")]
fn a_duplicate_key_stops_the_edition() {
    // given
    let builder = Edition::builder("test").notification_kind(kind("deploy.failed"));

    // when
    let _ = builder.notification_kind(kind("deploy.failed"));
}
