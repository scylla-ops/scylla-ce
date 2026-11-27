//! The details of the workspaces of a list: the first owner, the counts, the last activity and the
//! role of the caller.

use std::collections::HashMap;

use scylla_domain::authz::Object;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::workspaces::lists::{WorkspaceDetails, WorkspaceRoles};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Gate;
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Returns the details of each workspace, in the order of the ids. `with_email` gives the primary
/// email of the owners: only for a caller that sees each workspace of the instance. A failed read
/// of the roles of the schema counts only the owners and the roles that are rows.
pub(super) async fn details(
    pool: &PgPool,
    gate: &dyn Gate,
    caller: ActorId,
    ids: &[WorkspaceId],
    with_email: bool,
) -> Result<Vec<WorkspaceDetails>, Error> {
    // Read the counts, the owners and the roles that are rows.
    let mut tx = pool.begin().await?;
    let mut facts = store::facts(&mut tx, ids).await?;
    let owners = store::owners_of(&mut tx, ids, with_email).await?;
    let rows = store::row_roles_of(&mut tx, ids).await?;
    tx.commit().await?;

    // Put the roles of each workspace together, with the roles of the schema.
    let mut roles: HashMap<WorkspaceId, WorkspaceRoles> = HashMap::new();
    let mut first_owners = HashMap::new();
    for owned in owners {
        roles
            .entry(owned.workspace)
            .or_default()
            .owners
            .push(owned.owner.user);
        first_owners.entry(owned.workspace).or_insert(owned.owner);
    }
    for (workspace, actor, role) in rows {
        roles.entry(workspace).or_default().rows.push((actor, role));
    }
    for &id in ids {
        match gate.all_roles(&Object::of(id)).await {
            Ok(holders) => roles.entry(id).or_default().relations.extend(
                holders
                    .into_iter()
                    .map(|holder| (holder.actor, holder.role)),
            ),
            Err(error) => {
                tracing::warn!(%error, workspace = %id, "a lookup failed: the count shows the rows only");
            }
        }
    }

    // Make the details of each workspace.
    Ok(ids
        .iter()
        .map(|&id| {
            let roles = roles.remove(&id).unwrap_or_default();
            let facts = facts.remove(&id);
            WorkspaceDetails {
                workspace: id,
                owner: first_owners.remove(&id),
                member_count: roles.member_count(),
                project_count: facts
                    .as_ref()
                    .map_or(0, |facts| u32::try_from(facts.projects).unwrap_or(u32::MAX)),
                last_activity_at: facts.and_then(|facts| facts.last_activity_at),
                role: roles.role_of(caller),
            }
        })
        .collect())
}
