//! A person in a workspace: a role on a project needs a role in the workspace, and the roles on
//! the projects go with the last role in the workspace.

use scylla_domain::authz::Resource;
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users;
use scylla_domain::projects::Project;
use scylla_domain::tenancy::catalog::Level;
use scylla_domain::tenancy::roles::RoleId;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{Gate, Grants, Hooks, Rejection};
use sqlx::PgPool;

use super::give::scope_id;
use super::{held, service, store};
use crate::features::identity::actors;
use crate::features::tenancy::roles;
use crate::Error;

/// Returns `true` if an actor has a role in a workspace: an owner, a role of the schema, or a role
/// that is a row.
pub(super) async fn has_workspace_role(
    pool: &PgPool,
    gate: &dyn Gate,
    actor: ActorId,
    workspace: WorkspaceId,
) -> Result<bool, Error> {
    let mut tx = pool.begin().await?;
    let owner = store::owner_since(&mut tx, workspace, actor).await?;
    let rows = store::rows_of(&mut tx, actor, &[workspace.to_db()]).await?;
    tx.commit().await?;
    if owner.is_some() || !rows.is_empty() {
        return Ok(true);
    }
    let id = workspace.to_string();
    let roles = gate.roles_of(actor, Workspace::NAME).await?;
    Ok(roles.iter().any(|role| role.object == id))
}

/// Refuses a role on a project for a user with no role in the workspace: `NO_WORKSPACE_ROLE`.
pub(super) async fn check_in_workspace(
    pool: &PgPool,
    gate: &dyn Gate,
    actor: ActorId,
    workspace: WorkspaceId,
) -> Result<(), Error> {
    if has_workspace_role(pool, gate, actor, workspace).await? {
        return Ok(());
    }
    let message = "a person needs a role in the workspace before a role on a project";
    Err(Error::Rejected(Rejection::new(
        "NO_WORKSPACE_ROLE",
        message,
    )))
}

/// What the end of a membership needs.
pub(crate) struct Leaving<'a> {
    /// The database pool.
    pub(crate) pool: &'a PgPool,
    /// The access gate.
    pub(crate) gate: &'a dyn Gate,
    /// The kinds of objects with roles.
    pub(crate) grants: &'a Grants,
    /// The hooks of the edition.
    pub(crate) hooks: &'a Hooks,
}

/// Takes the roles on the projects of a workspace from a user with no role left in the workspace.
/// A user that keeps a role, and an app, keep their roles on the projects.
pub(crate) async fn end_membership(
    leaving: &Leaving<'_>,
    by: Actor,
    actor: ActorId,
    workspace: WorkspaceId,
) -> Result<(), Error> {
    let (pool, gate) = (leaving.pool, leaving.gate);
    if has_workspace_role(pool, gate, actor, workspace).await? {
        return Ok(());
    }
    let found = actors::find(pool, actor).await?;
    if found.kind != users::KIND {
        return Ok(());
    }

    // Find the roles of the user on the projects of the workspace.
    let on_workspace: Vec<_> = held::of_actor(pool, gate, actor, workspace)
        .await?
        .into_iter()
        .filter(|role| role.scope.level == Level::Project)
        .collect();
    let Some(kind) = leaving.grants.get(Project::NAME) else {
        return Ok(());
    };

    // Delete each one where it lives, then run the hooks of a role that goes.
    for role in on_workspace {
        let id = role.scope.id.as_str();
        let scope = (Project::NAME, scope_id(id)?);
        match RoleId::parse(&role.role) {
            Ok(row) => {
                let mut tx = pool.begin().await?;
                roles::store::revoke_row(&mut tx, actor, row, scope).await?;
                tx.commit().await?;
            }
            Err(_) => {
                if let Some(relationship) = kind.relationship(&role.role, id, actor) {
                    gate.delete(&[relationship]).await?;
                }
                let mut tx = pool.begin().await?;
                store::forget_relation(&mut tx, actor, scope, &role.role).await?;
                tx.commit().await?;
            }
        }
        let object = format!("{}:{id}", Project::NAME);
        let member = (actor, object, role.role.as_str());
        service::leave(pool, leaving.hooks, by.clone(), workspace, member).await?;
        tracing::info!(event = "role.revoked", role = role.role, object = id, grantee = %actor);
    }
    Ok(())
}
