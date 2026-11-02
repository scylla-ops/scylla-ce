//! The write of relationships with a transaction: SpiceDB first, then the commit, or the undo.

use scylla_domain::authz::Relationship;
use scylla_extension::{Gate, Tx};

use crate::Error;

/// Writes the relationships, then commits. If the commit fails, deletes the relationships
/// again, so the two stores agree.
pub async fn commit(tx: Tx<'_>, gate: &dyn Gate, grants: &[Relationship]) -> Result<(), Error> {
    gate.write(grants).await?;
    if let Err(error) = tx.commit().await {
        if let Err(undo) = gate.delete(grants).await {
            tracing::error!(%undo, "relationships stay after a failed commit");
        }
        return Err(error.into());
    }
    Ok(())
}
