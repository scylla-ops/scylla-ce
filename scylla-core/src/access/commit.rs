//! The commit of a transaction that wrote relations to SpiceDB before.

use scylla_extension::{Gate, Relation, Tx};

use crate::Error;

/// Commits, or deletes the relations again if the commit fails, so the two stores agree.
pub async fn commit(tx: Tx<'_>, gate: &dyn Gate, written: &[Relation]) -> Result<(), Error> {
    if let Err(error) = tx.commit().await {
        if let Err(undo) = gate.delete(written).await {
            tracing::error!(%undo, "relations stay after a failed commit");
        }
        return Err(error.into());
    }
    Ok(())
}
