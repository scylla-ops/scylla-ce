//! The effective access of an actor on an object: each permission of the catalog that it has, and
//! the roles that give it.

use std::collections::HashMap;

use scylla_domain::authz::{Resource, Schema};
use scylla_domain::projects::Project;
use scylla_domain::tenancy::catalog::{self, Level};
use scylla_domain::tenancy::grants::{HeldRole, OWNER};
use scylla_domain::tenancy::workspaces::Workspace;
use uuid::Uuid;

/// One permission of the catalog that an actor has, with the roles that give it.
pub(super) struct Effective {
    /// The id of the permission.
    pub(super) permission: &'static str,
    /// The roles that give it.
    pub(super) sources: Vec<HeldRole>,
}

/// Returns `true` if a role reaches the object: on the workspace, or on the project itself.
fn reaches(role: &HeldRole, level: Level, object: &str) -> bool {
    role.scope.level == Level::Workspace || (level == Level::Project && role.scope.id == object)
}

/// Returns the ids of the permissions that a role gives. `rows` holds the permissions of the roles
/// that are rows: a role of a level holds what they need at that level.
fn given(schema: &Schema, role: &HeldRole, rows: &HashMap<Uuid, Vec<String>>) -> Vec<&'static str> {
    if role.key == OWNER {
        return catalog::areas()
            .filter(|area| !area.machine)
            .flat_map(|area| area.permissions)
            .map(|permission| permission.id)
            .collect();
    }
    if let Some(permissions) = Uuid::parse_str(&role.key).ok().and_then(|id| rows.get(&id)) {
        return catalog::held(role.scope.level, permissions.iter().map(String::as_str));
    }
    let definition = match role.scope.level {
        Level::Project => Project::NAME,
        _ => Workspace::NAME,
    };
    catalog::given_by(schema, definition, &role.role, role.scope.level)
}

/// Returns the permissions of the catalog on an object of a level, in the order of the catalog,
/// that the roles give.
pub(super) fn effective(
    schema: &Schema,
    held: &[HeldRole],
    rows: &HashMap<Uuid, Vec<String>>,
    level: Level,
    object: &str,
) -> Vec<Effective> {
    let reaching: Vec<(&HeldRole, Vec<&'static str>)> = held
        .iter()
        .filter(|role| reaches(role, level, object))
        .map(|role| (role, given(schema, role, rows)))
        .collect();
    catalog::permissions_of(level)
        .filter_map(|permission| {
            let sources: Vec<HeldRole> = reaching
                .iter()
                .filter(|(_, ids)| ids.contains(&permission.id))
                .map(|(role, _)| (*role).clone())
                .collect();
            (!sources.is_empty()).then_some(Effective {
                permission: permission.id,
                sources,
            })
        })
        .collect()
}
