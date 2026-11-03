//! The deletion of a user: its account rows and its workspace ownerships in one transaction,
//! then its roles in SpiceDB. Its actor stays, so each `created_by` stays valid.

use scylla_domain::authz::Object;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Rejection, Tx};

use super::{store, AccountContext};
use crate::features::identity::admins;
use crate::features::tenancy::workspaces;
use crate::Error;

/// Deletes a user. A workspace where the user is the only owner goes with the user if it is
/// empty: no project, no agent, and no role of another actor. Else the deletion stops with
/// `LAST_OWNER`, and names these workspaces. The last active instance admin stays.
pub(crate) async fn delete_user(ctx: &AccountContext, user: ActorId) -> Result<(), Error> {
    // Lock the user, and keep an active instance admin.
    let mut tx = ctx.pool.begin().await?;
    if !store::lock_user(&mut tx, user).await? {
        return Err(Error::NotFound("user"));
    }
    admins::keep_an_admin(&mut tx, user).await?;

    // Find the workspaces of the user alone, and refuse those that are not empty.
    let sole = workspaces::lock_sole_owned(&mut tx, user).await?;
    let mut full = Vec::new();
    for workspace in &sole {
        if !is_empty(ctx, &mut tx, *workspace, user).await? {
            full.push(workspace.to_string());
        }
    }
    if !full.is_empty() {
        let message = "the user is the last owner of workspaces that are not empty";
        let rejection =
            Rejection::new("LAST_OWNER", message).detail("workspace_ids", full.join(","));
        return Err(Error::Rejected(rejection));
    }

    // Delete the empty workspaces, the ownerships and the user.
    workspaces::delete_all(&mut tx, &sole).await?;
    workspaces::leave_all(&mut tx, user).await?;
    store::delete_user(&mut tx, user).await?;
    tx.commit().await?;

    // Remove the roles of the user and of its workspaces. A failure goes to the log only.
    if let Err(error) = ctx.gate.delete_subject(&Object::of(user)).await {
        tracing::warn!(%error, %user, "the roles of a deleted user stay");
    }
    for workspace in sole {
        if let Err(error) = ctx.gate.delete_object(&Object::of(workspace)).await {
            tracing::warn!(%error, %workspace, "the roles of a deleted workspace stay");
        }
    }
    Ok(())
}

/// Returns `true` if a workspace has no project, no agent, and no role of an actor other than
/// the user.
async fn is_empty(
    ctx: &AccountContext,
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    user: ActorId,
) -> Result<bool, Error> {
    if workspaces::has_content(tx, workspace).await? {
        return Ok(false);
    }
    let holders = ctx.gate.subjects(&Object::of(workspace)).await?;
    Ok(holders.iter().all(|holder| *holder == user))
}
