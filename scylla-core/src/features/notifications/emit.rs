//! What records a fact: in the transaction of a write, and the notifier of the plugins.

use scylla_extension::{async_trait, Event, NotificationError, Notifier, Tx};
use sqlx::PgPool;
use uuid::Uuid;

use super::store::{events, rows};
use crate::Error;

/// The largest size of the parameters of a fact, in bytes.
const MAX_PARAMS: usize = 2048;

/// The name of the parameter with the snapshot of the name of the workspace.
pub(crate) const WORKSPACE_NAME: &str = "workspace_name";

/// The name of the parameter with the snapshot of the name of the project.
pub(crate) const PROJECT_NAME: &str = "project_name";

/// Records a fact in the transaction of a write, with a snapshot of the names of its workspace and
/// of its project. Returns `false` if its dedupe key has a record. A rollback removes the fact.
pub(crate) async fn emit(tx: &mut Tx<'_>, mut event: Event) -> Result<bool, Error> {
    // Add the names that a reader with no access still reads.
    if event.workspace.is_some() || event.project.is_some() {
        let names = events::names(tx, event.workspace, event.project).await?;
        if let Some(name) = names.workspace {
            event.params.insert(WORKSPACE_NAME.to_owned(), name);
        }
        if let Some(name) = names.project {
            event.params.insert(PROJECT_NAME.to_owned(), name);
        }
    }

    // Refuse parameters that are too large, then record the fact.
    let size: usize = event
        .params
        .iter()
        .map(|(key, value)| key.len() + value.len())
        .sum();
    if size > MAX_PARAMS {
        let reason = format!("the parameters of {} are too large", event.kind);
        return Err(Error::Internal(reason));
    }
    events::insert(tx, &event).await
}

/// Deletes each row of an object, and each of its facts that wait.
pub(crate) async fn withdraw(tx: &mut Tx<'_>, subject_kind: &str, id: Uuid) -> Result<(), Error> {
    events::delete_pending(tx, subject_kind, id).await?;
    rows::delete_subject(tx, subject_kind, id).await
}

/// Closes the rows of a kind on an object: the reader answered, so they wait for no action and
/// count as read.
pub(crate) async fn close(
    tx: &mut Tx<'_>,
    kind: &str,
    subject_kind: &str,
    id: Uuid,
) -> Result<(), Error> {
    rows::close_subject(tx, kind, subject_kind, id).await
}

/// Opens the rows of a kind on an object again: they wait for an action again.
pub(crate) async fn reopen(
    tx: &mut Tx<'_>,
    kind: &str,
    subject_kind: &str,
    id: Uuid,
) -> Result<(), Error> {
    rows::reopen_subject(tx, kind, subject_kind, id).await
}

/// The notifier that the core gives to the plugins.
pub(crate) struct PgNotifier {
    /// The database pool.
    pub(crate) pool: PgPool,
}

/// Makes the error of the contract.
fn failed(error: Error) -> NotificationError {
    NotificationError(error.to_string())
}

#[async_trait]
impl Notifier for PgNotifier {
    async fn emit(&self, tx: &mut Tx<'_>, event: Event) -> Result<bool, NotificationError> {
        emit(tx, event).await.map_err(failed)
    }

    async fn emit_detached(&self, event: Event) -> Result<bool, NotificationError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|error| failed(error.into()))?;
        let emitted = emit(&mut tx, event).await.map_err(failed)?;
        tx.commit().await.map_err(|error| failed(error.into()))?;
        Ok(emitted)
    }
}
