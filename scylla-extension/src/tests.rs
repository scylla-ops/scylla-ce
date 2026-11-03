//! Unit tests of the contract that the API cannot show: the log line of a refusal.

use std::io;
use std::sync::{Arc, Mutex};

use scylla_domain::identity::actors::{Actor, ActorId, ActorKind};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};

use crate::{AccessError, Operation, Principal};

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

/// A refusal logs one WARN line with the actor, the route, the permission and the object.
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
    let principal = Principal::new(actor.clone(), operation, Arc::new(()), None);
    let workspace = WorkspaceId::generate();
    let check = Workspace::GET_WORKSPACE.on(workspace);

    // when
    let error =
        tracing::subscriber::with_default(subscriber, || AccessError::denied(&principal, &check));

    // then
    let text = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
    assert_eq!(text.lines().count(), 1, "{text}");
    assert!(text.contains(" WARN "), "{text}");
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
