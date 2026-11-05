//! The holders of a role: `ListRoleHolders`.

use scylla_domain::authz::Resource;
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::catalog::Level;
use scylla_domain::tenancy::grants::{AccessEntry, GrantScope, HeldRole};
use scylla_domain::tenancy::roles::{builtin, BuiltinSource, RoleId, RoleRow};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Gate;
use sqlx::PgPool;

use super::store;
use crate::features::tenancy::grants::held;
use crate::Error;

/// The largest number of holders in one answer.
const MAX_HOLDERS: usize = 100;

/// Returns `true` if a custom role is in the list of the roles of an owner.
fn in_list(row: &RoleRow, workspace: Option<WorkspaceId>) -> bool {
    match workspace {
        None => row.workspace.is_none(),
        Some(_) => {
            row.workspace == workspace || (row.workspace.is_none() && row.level != Level::Platform)
        }
    }
}

/// Returns `true` if a key names a role of the list of an owner.
pub(super) async fn known(
    pool: &PgPool,
    key: &str,
    workspace: Option<WorkspaceId>,
) -> Result<bool, Error> {
    if let Some(role) = builtin(key) {
        return Ok(workspace.is_none() || role.level != Level::Platform);
    }
    let Ok(id) = RoleId::parse(key) else {
        return Ok(false);
    };
    let row = super::service::owner(pool, id).await?;
    Ok(row.is_some_and(|row| in_list(&row, workspace)))
}

/// Returns the grants of a role on the whole instance.
async fn on_instance(pool: &PgPool, gate: &dyn Gate, key: &str) -> Result<Vec<HeldRole>, Error> {
    if let Some(role) = builtin(key) {
        return match role.source {
            BuiltinSource::InstanceAdmins => held::instance_admins(pool).await,
            BuiltinSource::WorkspaceOwners => held::all_owners(pool).await,
            BuiltinSource::Relation(definition, relation) => {
                let level = role.level;
                let grants = gate.holders_of(definition, relation).await?;
                let mut tx = pool.begin().await?;
                let names = store::scope_names(&mut tx, level.as_str()).await?;
                tx.commit().await?;
                Ok(grants
                    .into_iter()
                    .map(|grant| HeldRole {
                        scope: GrantScope {
                            level,
                            name: names.get(&grant.object).cloned().unwrap_or_default(),
                            id: grant.object,
                        },
                        actor: grant.actor,
                        role: relation.to_owned(),
                        key: key.to_owned(),
                        name: role.name.to_owned(),
                        granted_at: None,
                        granted_by: None,
                    })
                    .collect())
            }
        };
    }
    let Ok(id) = RoleId::parse(key) else {
        return Ok(Vec::new());
    };
    let mut tx = pool.begin().await?;
    let row = store::find(&mut tx, id).await?;
    let grants = store::grants_of_role(&mut tx, id).await?;
    let names =
        store::scope_names(&mut tx, row.as_ref().map_or("", |row| row.level.as_str())).await?;
    tx.commit().await?;
    let Some(row) = row else {
        return Ok(Vec::new());
    };
    Ok(grants
        .into_iter()
        .map(|(actor, kind, scope)| {
            let text = if kind == Platform::NAME {
                Platform::ID.to_string()
            } else {
                scope.to_string()
            };
            HeldRole {
                scope: GrantScope {
                    level: Level::parse(&kind).unwrap_or(Level::Project),
                    name: names.get(&text).cloned().unwrap_or_default(),
                    id: text,
                },
                actor,
                role: key.to_owned(),
                key: key.to_owned(),
                name: row.name.clone(),
                granted_at: None,
                granted_by: None,
            }
        })
        .collect())
}

/// Returns the grants of a role, at most `MAX_HOLDERS` in name order, with the number of grants.
pub(super) async fn list(
    pool: &PgPool,
    gate: &dyn Gate,
    key: &str,
    workspace: Option<WorkspaceId>,
) -> Result<(Vec<(AccessEntry, HeldRole)>, usize), Error> {
    let grants: Vec<HeldRole> = match workspace {
        Some(workspace) => {
            let all = held::in_workspace(pool, gate, workspace).await?;
            all.into_iter().filter(|role| role.key == key).collect()
        }
        None => on_instance(pool, gate, key).await?,
    };
    let total = grants.len();
    let members = held::members(pool, grants).await?;
    let entries = members
        .into_iter()
        .flat_map(|member| {
            let profile = member.profile;
            member
                .roles
                .into_iter()
                .map(move |role| (profile.clone(), role))
        })
        .take(MAX_HOLDERS)
        .collect();
    Ok((entries, total))
}
