//! The logic of `AccessService`: the actors that can get a role, and the pages of a list of the
//! access.

use scylla_domain::authz::Object;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::apps;
use scylla_domain::tenancy::grants::{AccessCursor, AccessEntry, AccessPage};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageSize;
use scylla_extension::{Gate, Rejection};
use sqlx::PgPool;

use super::store;
use crate::features::identity::{actors, apps as app_feature};
use crate::Error;

/// Passes an actor that can get a role in a workspace: an actor that exists, and that is not an
/// App of another workspace. An unknown actor gives `NotFound("actor")`, and an App of another
/// workspace `ACTOR_OUTSIDE_WORKSPACE`.
pub(super) async fn check_actor(
    pool: &PgPool,
    actor: ActorId,
    workspace: WorkspaceId,
) -> Result<(), Error> {
    let actor = actors::find(pool, actor).await?;
    if actor.kind != apps::KIND
        || app_feature::workspace_of(pool, actor.id).await? == Some(workspace)
    {
        return Ok(());
    }
    let message = "an app gets roles only in its own workspace";
    Err(Error::Rejected(Rejection::new(
        "ACTOR_OUTSIDE_WORKSPACE",
        message,
    )))
}

/// Returns one page of the access to an object: the owners of a workspace first, in user id
/// order, then the roles, in the order of SpiceDB. Returns `None` for a token that does not fit
/// the object, or that SpiceDB refuses.
pub(super) async fn list_access(
    pool: &PgPool,
    gate: &dyn Gate,
    object: &Object,
    owners_of: Option<WorkspaceId>,
    start: Option<AccessCursor>,
    size: PageSize,
) -> Result<Option<AccessPage>, Error> {
    // Find where the page starts: in the owners of a workspace, or in the roles.
    let (owners, cursor) = match (start, owners_of) {
        (None, Some(workspace)) => (Some((workspace, None)), None),
        (None, None) => (None, None),
        (Some(AccessCursor::Owners(after)), Some(workspace)) => {
            (Some((workspace, Some(after))), None)
        }
        (Some(AccessCursor::Owners(_)), None) => return Ok(None),
        (Some(AccessCursor::Roles(cursor)), _) => (None, cursor),
    };

    // Read the owners. A full page of owners ends the page.
    let mut tx = pool.begin().await?;
    let mut entries = Vec::new();
    if let Some((workspace, after)) = owners {
        entries = store::owners(&mut tx, workspace, after, size.get() + 1).await?;
        if entries.len() > size.get() {
            entries.truncate(size.get());
            let next = entries
                .last()
                .map(|owner| AccessCursor::Owners(owner.actor_id));
            return Ok(Some(AccessPage { entries, next }));
        }
    }

    // Fill the page with the roles, and the profiles of their actors.
    let count = size.get() - entries.len();
    let Some(roles) = gate.roles(object, cursor.as_deref(), count).await? else {
        return Ok(None);
    };
    let ids: Vec<_> = roles.holders.iter().map(|holder| holder.actor).collect();
    let profiles = store::profiles(&mut tx, &ids).await?;
    tx.commit().await?;
    for holder in roles.holders {
        let profile = profiles
            .iter()
            .find(|profile| profile.actor_id == holder.actor);
        if let Some(profile) = profile {
            entries.push(AccessEntry {
                role: holder.role,
                ..profile.clone()
            });
        }
    }
    let next = roles
        .more
        .then(|| AccessCursor::Roles(roles.cursor.or(cursor)));
    Ok(Some(AccessPage { entries, next }))
}
