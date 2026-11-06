//! `Grant` and `Revoke` on a workspace or on a project: a relation of the schema or a custom role,
//! the holders of the role, and what the caller must hold to give it.

use scylla_domain::authz::{Object, Relationship, Resource};
use scylla_domain::identity::actors::{ActorId, ActorKind};
use scylla_domain::identity::users;
use scylla_domain::projects::Project;
use scylla_domain::tenancy::catalog::{self, Level};
use scylla_domain::tenancy::roles::{BuiltinRole, Holder, RoleId, RoleRow};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{Gate, Grantable, Principal};
use sqlx::PgPool;
use uuid::Uuid;

use super::grpc::GrantsGrpc;
use super::store;
use crate::features::notifications::{self, kinds};
use crate::features::tenancy::roles;
use crate::Error;

/// A role by its name in `Grant`, before the read of a custom role.
pub(super) enum Named {
    /// A relation of the schema.
    Relation(Relationship),
    /// The id of a custom role.
    Row(RoleId),
}

/// A role to give or to take back.
pub(super) enum Given {
    /// A relation of the schema, with its name.
    Relation(Relationship, String),
    /// A custom role.
    Row(RoleRow),
}

/// Returns the level of the roles of a kind of object.
fn level_of(kind: &str) -> Option<Level> {
    match kind {
        Workspace::NAME => Some(Level::Workspace),
        Project::NAME => Some(Level::Project),
        _ => None,
    }
}

/// Reads the name of a role: a relation of the kind, else the id of a custom role on a workspace or
/// on a project. Another name is a request error.
pub(super) fn read_name(
    kind: &Grantable,
    name: &str,
    id: &str,
    role: &str,
    actor: ActorId,
) -> Result<Named, Error> {
    if let Some(relationship) = kind.relationship(role, id, actor) {
        return Ok(Named::Relation(relationship));
    }
    if level_of(name).is_some() {
        if let Ok(id) = RoleId::parse(role) {
            return Ok(Named::Row(id));
        }
    }
    Err(Error::invalid("role", format!("is not a role of {name}")))
}

/// Refuses a role that the actor cannot hold: a role for apps and a user, or a custom role for the
/// other kind of actor.
pub(super) fn check_holder(
    given: &Given,
    kind: &str,
    role: &str,
    actor: &ActorKind,
) -> Result<(), Error> {
    let holder = match given {
        Given::Relation(..) => match BuiltinRole::of_relation(kind, role) {
            Some(BuiltinRole {
                holder: Holder::Apps,
                ..
            }) => Holder::Apps,
            _ => return Ok(()),
        },
        Given::Row(row) => row.holder,
    };
    let fits = match holder {
        Holder::Apps => *actor != users::KIND,
        Holder::People => *actor == users::KIND,
    };
    if fits {
        return Ok(());
    }
    let reason = format!("is for {} only", holder.as_str());
    Err(Error::invalid("role", reason))
}

/// Returns the ids of the permissions that a role gives.
fn permissions_of(
    given: &Given,
    schema: &scylla_domain::authz::Schema,
    kind: &str,
    role: &str,
) -> Vec<&'static str> {
    let Some(level) = level_of(kind) else {
        return Vec::new();
    };
    match given {
        Given::Relation(..) => catalog::given_by(schema, kind, role, level),
        Given::Row(row) => row
            .permissions
            .iter()
            .filter_map(|id| catalog::find(id).map(|(_, permission)| permission.id))
            .collect(),
    }
}

impl GrantsGrpc {
    /// Reads a custom role that the workspace can use, on the level of the object.
    pub(super) async fn resolve(
        &self,
        named: Named,
        kind: &str,
        workspace: WorkspaceId,
    ) -> Result<Given, Error> {
        match named {
            Named::Relation(relationship) => {
                let name = relationship.relation().to_owned();
                Ok(Given::Relation(relationship, name))
            }
            Named::Row(id) => {
                let not_here = || Error::invalid("role", format!("is not a role of {kind}"));
                let row = roles::service::owner(&self.pool, id)
                    .await?
                    .ok_or_else(not_here)?;
                let usable = row.workspace.is_none() || row.workspace == Some(workspace);
                if !usable || Some(row.level) != level_of(kind) {
                    return Err(not_here());
                }
                Ok(Given::Row(row))
            }
        }
    }

    /// Refuses a role with a permission that the caller does not hold on the object: `ESCALATION`.
    pub(super) async fn check_can_give(
        &self,
        caller: &Principal,
        given: &Given,
        (kind, id): (&str, &str),
        workspace: WorkspaceId,
        role: &str,
    ) -> Result<(), Error> {
        let grantable = self
            .grants
            .get(kind)
            .ok_or_else(|| Error::invalid("object.kind", "has no roles"))?;
        let object: Object = grantable
            .object(id)
            .ok_or_else(|| Error::invalid("object.id", "is not an id"))?;
        let permissions = permissions_of(given, &self.schema, kind, role);
        let gate = self.gate.as_ref();
        roles::service::check_escalation(gate, caller, &object, Some(workspace), &permissions).await
    }
}

/// Reads the id of a scope.
pub(super) fn scope_id(id: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(id).map_err(|_| Error::invalid("object.id", "is not an id"))
}

/// Writes a role where it lives, with its time and its author, and records the fact.
pub(super) async fn write(
    pool: &PgPool,
    gate: &dyn Gate,
    given: Given,
    (kind, id): (&str, &str),
    actor: ActorId,
    by: ActorId,
    workspace: WorkspaceId,
) -> Result<(), Error> {
    let fact = |role: &str| {
        kinds::role_change(kinds::ROLE_GRANTED, role, (kind, id), workspace, actor, by)
    };
    match given {
        Given::Relation(relationship, name) => {
            gate.write(&[relationship]).await?;
            let mut tx = pool.begin().await?;
            if level_of(kind).is_some() {
                let scope = (kind, scope_id(id)?);
                store::note_relation(&mut tx, actor, scope, &name, by).await?;
            }
            notifications::emit(&mut tx, fact(&name)).await?;
            tx.commit().await?;
        }
        Given::Row(row) => {
            let mut tx = pool.begin().await?;
            roles::store::grant_row(&mut tx, actor, row.id, (kind, scope_id(id)?), by).await?;
            notifications::emit(&mut tx, fact(&row.name)).await?;
            tx.commit().await?;
        }
    }
    Ok(())
}

/// Deletes a role where it lives, with its time and its author, and records the fact. A role that
/// the actor does not have is not an error.
pub(super) async fn erase(
    pool: &PgPool,
    gate: &dyn Gate,
    given: Given,
    (kind, id): (&str, &str),
    actor: ActorId,
    by: ActorId,
    workspace: WorkspaceId,
) -> Result<(), Error> {
    let fact = |role: &str| {
        kinds::role_change(kinds::ROLE_REVOKED, role, (kind, id), workspace, actor, by)
    };
    match given {
        Given::Relation(relationship, name) => {
            gate.delete(&[relationship]).await?;
            let mut tx = pool.begin().await?;
            if level_of(kind).is_some() {
                let scope = (kind, scope_id(id)?);
                store::forget_relation(&mut tx, actor, scope, &name).await?;
            }
            notifications::emit(&mut tx, fact(&name)).await?;
            tx.commit().await?;
        }
        Given::Row(row) => {
            let mut tx = pool.begin().await?;
            roles::store::revoke_row(&mut tx, actor, row.id, (kind, scope_id(id)?)).await?;
            notifications::emit(&mut tx, fact(&row.name)).await?;
            tx.commit().await?;
        }
    }
    Ok(())
}
