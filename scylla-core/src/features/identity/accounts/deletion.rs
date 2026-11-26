//! The deletion of a user: its account rows, the image of its avatar, its workspace ownerships, its
//! personal tokens and the rows of the edition in one transaction, then its roles in SpiceDB. Its
//! actor stays, so each `created_by` stays valid, and its acceptances of the terms stay as the
//! proof, with the hash of its email.

use scylla_domain::authz::Object;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Rejection, Tx, UserContext};

use super::{store, AccountContext};
use crate::features::identity::{admins, api_tokens, terms};
use crate::features::platform::images;
use crate::features::tenancy::{grants, roles, workspaces};
use crate::Error;

/// Deletes a user for an actor: the user itself or an instance admin. A workspace where the user
/// is the only owner goes with the user if it is empty: no project, no App, and no role of
/// another actor. Else the deletion stops with `LAST_OWNER`, and names these workspaces. The last
/// active instance admin stays. The `AfterDeleteUser` hooks run before the commit.
pub(crate) async fn delete_user(
    ctx: &AccountContext,
    by: ActorId,
    user: ActorId,
) -> Result<(), Error> {
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

    // Delete the rows of the user, and keep the proof of its acceptances of the terms.
    let email = store::find_user(&mut tx, user)
        .await?
        .and_then(|found| found.email);
    workspaces::delete_all(&mut tx, &sole).await?;
    workspaces::leave_all(&mut tx, user).await?;
    api_tokens::delete_all(&mut tx, user).await?;
    roles::delete_grants_of(&mut tx, user).await?;
    terms::keep_as_proof(&mut tx, user, email.as_deref()).await?;
    if let Some(image) = store::delete_user(&mut tx, user).await? {
        images::store::delete(&mut tx, image).await?;
    }

    // Let the edition remove its rows of the user, then commit.
    let hook = UserContext::new(user, by);
    ctx.hooks.after_delete_user(&mut tx, &hook).await?;
    tx.commit().await?;
    tracing::info!(event = "user.deleted", %user, workspaces = sole.len());

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

/// Returns `true` if a workspace has no project, no App, and no role of an actor other than
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
    if grants::store::others_on_scope(tx, workspace.to_db(), user).await? {
        return Ok(false);
    }
    let holders = ctx.gate.subjects(&Object::of(workspace)).await?;
    Ok(holders.iter().all(|holder| *holder == user))
}
