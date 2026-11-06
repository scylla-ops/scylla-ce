//! The logic of the roles: the list with the built-in roles, the changes of the custom roles, and
//! the roles of the instance that an actor holds.

use std::collections::BTreeMap;

use scylla_domain::authz::{Object, Resource, Schema};
use scylla_domain::projects::Project;
use scylla_domain::tenancy::catalog::{self, Level};
use scylla_domain::tenancy::roles::{
    BuiltinRole, BuiltinSource, Holder, NewRole, Role, RoleId, RoleRow, BUILTIN_ROLES,
};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_domain::Name;
use scylla_extension::{Gate, Principal, Tx};
use sqlx::PgPool;

use super::store;
use crate::features::identity::accounts::refused;
use crate::features::tenancy::grants;
use crate::Error;

/// The number of grants of the roles of the schema: by definition, then by relation.
type RoleCounts = BTreeMap<&'static str, BTreeMap<String, u32>>;

/// The fields of a custom role that a client can change.
pub(super) struct RoleFields {
    /// The name.
    pub(super) name: Name,
    /// What the role does.
    pub(super) description: String,
    /// The ids of the permissions.
    pub(super) permissions: Vec<String>,
}

/// The error of a change that a role with a holder refuses.
fn role_in_use() -> Error {
    refused("ROLE_IN_USE", "an actor holds this role")
}

/// Returns the number of permissions that gives full control of a level.
fn full_count(level: Level) -> usize {
    catalog::areas()
        .filter(|area| !area.machine && level.covers(area.level))
        .map(|area| area.permissions.len())
        .sum()
}

/// Returns `true` if a role has each permission of its level.
fn is_full(level: Level, permissions: &[&'static str]) -> bool {
    permissions.len() == full_count(level)
}

/// Puts the permissions of a row in the order of the catalog.
fn in_catalog_order(level: Level, kept: &[String]) -> Vec<String> {
    catalog::permissions_of(level)
        .filter(|permission| kept.iter().any(|id| id == permission.id))
        .map(|permission| permission.id.to_owned())
        .collect()
}

/// Makes the value of a custom role.
fn custom(row: RoleRow) -> Role {
    let key = row.id.to_string();
    Role {
        permissions: in_catalog_order(row.level, &row.permissions),
        grant_name: key.clone(),
        key,
        name: row.name,
        description: row.description,
        level: row.level,
        holder: row.holder,
        builtin: false,
        full_control: false,
        holder_count: row.holder_count,
        workspace: row.workspace,
    }
}

/// Makes the value of a built-in role, from the schema and the facts of its holders.
async fn built_in(
    tx: &mut Tx<'_>,
    schema: &Schema,
    role: &BuiltinRole,
    counts: &RoleCounts,
) -> Result<Role, Error> {
    // Read the permissions and the holders from where the role lives.
    let (permissions, holder_count): (Vec<String>, u32) = match role.source {
        BuiltinSource::InstanceAdmins => {
            let all = catalog::areas()
                .filter(|area| !area.machine)
                .flat_map(|area| area.permissions)
                .map(|permission| permission.id.to_owned())
                .collect();
            (all, store::admin_count(tx).await?)
        }
        BuiltinSource::Relation(definition, relation) => {
            let given = catalog::given_by(schema, definition, relation, role.level);
            let count = counts
                .get(definition)
                .and_then(|counts| counts.get(relation))
                .copied()
                .unwrap_or(0);
            (given.iter().map(|id| (*id).to_owned()).collect(), count)
        }
    };

    // Make the value.
    let ids: Vec<&'static str> = catalog::permissions_of(role.level)
        .filter(|permission| permissions.iter().any(|id| id == permission.id))
        .map(|permission| permission.id)
        .collect();
    Ok(Role {
        key: role.key.to_owned(),
        name: role.name.to_owned(),
        description: role.description.to_owned(),
        level: role.level,
        holder: role.holder,
        grant_name: role.grant_name().to_owned(),
        builtin: true,
        full_control: role.holder == Holder::People && is_full(role.level, &ids),
        permissions,
        holder_count,
        workspace: None,
    })
}

/// Returns the number of grants of the roles of the schema.
async fn relation_counts(gate: &dyn Gate) -> Result<RoleCounts, Error> {
    let mut counts = RoleCounts::new();
    for definition in [Workspace::NAME, Project::NAME] {
        counts.insert(definition, gate.role_counts(definition).await?);
    }
    Ok(counts)
}

/// Returns the roles of the instance, or the roles for a workspace. The number of holders is for
/// the whole instance, or for the workspace and its projects in a list for a workspace.
pub(super) async fn list(
    pool: &PgPool,
    gate: &dyn Gate,
    schema: &Schema,
    workspace: Option<WorkspaceId>,
) -> Result<Vec<Role>, Error> {
    let counts = relation_counts(gate).await?;
    let mut tx = pool.begin().await?;

    // Read the built-in roles of the list, then the custom roles.
    let mut list = Vec::new();
    for role in BUILTIN_ROLES
        .iter()
        .filter(|role| workspace.is_none() || role.level != Level::Platform)
    {
        list.push(built_in(&mut tx, schema, role, &counts).await?);
    }
    let rows = store::list_custom(&mut tx, workspace).await?;
    tx.commit().await?;
    list.extend(rows.into_iter().map(custom));
    if let Some(workspace) = workspace {
        let held = grants::held::in_workspace(pool, gate, workspace).await?;
        for role in &mut list {
            let count = held.iter().filter(|one| one.key == role.key).count();
            role.holder_count = u32::try_from(count).unwrap_or(u32::MAX);
        }
    }
    Ok(list)
}

/// Returns the owner of a custom role, by the text of its id: `None` for an unknown role.
pub(crate) async fn owner(pool: &PgPool, id: RoleId) -> Result<Option<RoleRow>, Error> {
    let mut tx = pool.begin().await?;
    let row = store::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(row)
}

/// Checks and completes the permissions of a role.
pub(super) fn complete(
    level: Level,
    holder: Holder,
    ids: &[String],
) -> Result<Vec<&'static str>, Error> {
    catalog::complete(level, holder == Holder::Apps, ids)
        .map_err(|error| Error::invalid("permissions", error.to_string()))
}

/// Refuses a permission of a role that the caller does not hold on the object of the role:
/// `ESCALATION`. An implicit permission needs no check, and so does a permission of a machine: no
/// person holds it.
pub(crate) async fn check_escalation(
    gate: &dyn Gate,
    caller: &Principal,
    object: &Object,
    workspace: Option<WorkspaceId>,
    permissions: &[&'static str],
) -> Result<(), Error> {
    let asked: Vec<&'static str> = permissions
        .iter()
        .copied()
        .filter(|id| {
            catalog::find(id)
                .is_some_and(|(area, permission)| !area.machine && !permission.implicit)
        })
        .collect();
    let answers = gate.holds(caller, object, workspace, &asked).await?;
    let missing: Vec<&str> = asked
        .iter()
        .zip(answers)
        .filter(|(_, holds)| !holds)
        .map(|(id, _)| *id)
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let message = format!("you do not hold {}", missing.join(", "));
    Err(refused("ESCALATION", &message))
}

/// Makes a custom role, and returns it.
pub(super) async fn create(pool: &PgPool, role: NewRole) -> Result<Role, Error> {
    let mut tx = pool.begin().await?;
    store::insert(&mut tx, &role).await?;
    let row = store::find(&mut tx, role.id)
        .await?
        .ok_or(Error::NotFound("role"))?;
    tx.commit().await?;
    tracing::info!(event = "role.created", role = %role.id, level = role.level.as_str());
    Ok(custom(row))
}

/// Changes a custom role, and returns it. The change is live at once for each holder.
pub(super) async fn update(
    pool: &PgPool,
    id: RoleId,
    level: Level,
    fields: &RoleFields,
    permissions: &[&'static str],
) -> Result<Role, Error> {
    let mut tx = pool.begin().await?;
    let current = store::lock_level(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("role"))?;
    if level != current && store::has_grant(&mut tx, id).await? {
        return Err(role_in_use());
    }
    let text = (fields.name.as_str(), fields.description.as_str());
    store::update(&mut tx, id, level, text, permissions).await?;
    let row = store::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("role"))?;
    tx.commit().await?;
    tracing::info!(event = "role.updated", role = %id, role_level = level.as_str());
    Ok(custom(row))
}

/// Deletes a custom role. A role that an actor holds stays: `ROLE_IN_USE`.
pub(super) async fn delete(pool: &PgPool, id: RoleId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !store::delete_unused(&mut tx, id).await? {
        return Err(role_in_use());
    }
    tx.commit().await?;
    tracing::info!(event = "role.deleted", role = %id);
    Ok(())
}
