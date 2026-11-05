//! The values of the roles that an actor holds.

use std::collections::HashMap;

use scylla_domain::authz::Resource;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::Platform;
use scylla_domain::projects::Project;
use scylla_domain::tenancy::catalog::Level;
use scylla_domain::tenancy::grants::{GrantScope, HeldRole, OWNER};
use scylla_domain::tenancy::roles::{builtin, BuiltinRole, INSTANCE_ADMIN};
use scylla_domain::tenancy::workspaces::Workspace;
use time::OffsetDateTime;
use uuid::Uuid;

use super::store::RowGrant;

/// The time and the author of the grants of the schema, by actor, scope and relation.
pub(super) type Times = HashMap<(ActorId, Uuid, String), (OffsetDateTime, Option<ActorId>)>;

/// Makes the value of a role of the schema.
pub(super) fn relation_role(
    scope: &GrantScope,
    actor: ActorId,
    relation: String,
    times: &Times,
) -> HeldRole {
    let definition = match scope.level {
        Level::Project => Project::NAME,
        _ => Workspace::NAME,
    };
    let builtin = BuiltinRole::of_relation(definition, &relation);
    let time = Uuid::parse_str(&scope.id)
        .ok()
        .and_then(|id| times.get(&(actor, id, relation.clone())));
    HeldRole {
        scope: scope.clone(),
        actor,
        key: builtin.map_or_else(|| relation.clone(), |role| role.key.to_owned()),
        name: builtin.map_or_else(|| relation.clone(), |role| role.name.to_owned()),
        role: relation,
        granted_at: time.map(|time| time.0),
        granted_by: time.and_then(|time| time.1),
    }
}

/// Makes the value of a role that is a row.
pub(super) fn row_role(grant: RowGrant, scope: &GrantScope) -> HeldRole {
    HeldRole {
        scope: scope.clone(),
        actor: grant.actor,
        role: grant.role.to_string(),
        key: grant.role.to_string(),
        name: grant.name,
        granted_at: Some(grant.created_at),
        granted_by: grant.created_by,
    }
}

/// Makes the value of the role of an instance admin.
pub(super) fn admin_role(actor: ActorId, since: OffsetDateTime) -> HeldRole {
    let role = builtin(INSTANCE_ADMIN);
    HeldRole {
        scope: GrantScope {
            level: Level::Platform,
            id: Platform::ID.to_string(),
            name: String::new(),
        },
        actor,
        role: INSTANCE_ADMIN.to_owned(),
        key: INSTANCE_ADMIN.to_owned(),
        name: role.map_or_else(String::new, |role| role.name.to_owned()),
        granted_at: Some(since),
        granted_by: None,
    }
}

/// Makes the value of the role of an owner.
pub(super) fn owner_role(
    workspace: &GrantScope,
    actor: ActorId,
    since: OffsetDateTime,
) -> HeldRole {
    let role = builtin(OWNER);
    HeldRole {
        scope: workspace.clone(),
        actor,
        role: OWNER.to_owned(),
        key: OWNER.to_owned(),
        name: role.map_or_else(String::new, |role| role.name.to_owned()),
        granted_at: Some(since),
        granted_by: None,
    }
}

/// Returns the scope of a row among the known scopes.
pub(super) fn scope_of<'a>(scopes: &'a [&GrantScope], grant: &RowGrant) -> Option<&'a GrantScope> {
    scopes
        .iter()
        .find(|scope| Uuid::parse_str(&scope.id).ok() == Some(grant.scope_id))
        .copied()
}
