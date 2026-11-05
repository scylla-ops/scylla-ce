//! `DeleteRole` with a move: the holders get another role, then the role goes.

use scylla_domain::authz::{Object, Schema};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users;
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::catalog;
use scylla_domain::tenancy::roles::{BuiltinRole, BuiltinSource, RoleId, RoleRow, BUILTIN_ROLES};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Gate, Grants, Hooks, Principal};
use sqlx::PgPool;

use super::{service, store};
use crate::features::identity::actors;
use crate::features::tenancy::grants;
use crate::Error;

/// The role that the holders get.
enum Target {
    /// A role of the schema: the definition and the relation.
    Relation(&'static str, &'static str),
    /// A custom role.
    Row(RoleRow),
}

/// The tools of a move.
pub(super) struct Mover<'a> {
    /// The database pool.
    pub(super) pool: &'a PgPool,
    /// The access gate.
    pub(super) gate: &'a dyn Gate,
    /// The permission model.
    pub(super) schema: &'a Schema,
    /// The kinds of objects with roles.
    pub(super) grants: &'a Grants,
    /// The hooks of the edition, for a membership.
    pub(super) hooks: &'a Hooks,
}

/// The error of a role that the holders cannot get.
fn not_a_target() -> Error {
    Error::invalid("move_to", "is not a role that the holders can get")
}

impl Mover<'_> {
    /// Finds the role that the holders of a role get: the same level, the same holders, and a role
    /// that the owner of the old role can use.
    async fn target(&self, old: &RoleRow, name: &str) -> Result<Target, Error> {
        let built_in = BUILTIN_ROLES.iter().find(|role: &&BuiltinRole| {
            role.grant_name() == name && role.level == old.level && role.holder == old.holder
        });
        if let Some(role) = built_in {
            return match role.source {
                BuiltinSource::Relation(definition, relation) => {
                    Ok(Target::Relation(definition, relation))
                }
                BuiltinSource::InstanceAdmins => Err(not_a_target()),
            };
        }
        let id = RoleId::parse(name).map_err(|_| not_a_target())?;
        let row = service::owner(self.pool, id)
            .await?
            .ok_or_else(not_a_target)?;
        let usable = row.workspace.is_none() || row.workspace == old.workspace;
        if row.id == old.id || !usable || row.level != old.level || row.holder != old.holder {
            return Err(not_a_target());
        }
        Ok(Target::Row(row))
    }

    /// Refuses a target with a permission that the caller does not hold on the owner of the old
    /// role: `ESCALATION`.
    async fn check_can_give(
        &self,
        caller: &Principal,
        old: &RoleRow,
        target: &Target,
    ) -> Result<(), Error> {
        let permissions: Vec<&'static str> = match target {
            Target::Relation(definition, relation) => {
                catalog::given_by(self.schema, definition, relation, old.level)
            }
            Target::Row(row) => row
                .permissions
                .iter()
                .filter_map(|id| catalog::find(id).map(|(_, permission)| permission.id))
                .collect(),
        };
        let object = owner_object(old.workspace);
        service::check_escalation(self.gate, caller, &object, old.workspace, &permissions).await
    }

    /// Gives the holders of a role the role `name`, then deletes the role.
    pub(super) async fn delete_with_move(
        &self,
        caller: &Principal,
        old: &RoleRow,
        name: &str,
    ) -> Result<(), Error> {
        let target = self.target(old, name).await?;
        self.check_can_give(caller, old, &target).await?;

        // Read the grants, and give the new role on each scope.
        let mut tx = self.pool.begin().await?;
        let holders = store::grants_of_role(&mut tx, old.id).await?;
        tx.commit().await?;
        let (old_name, new_name) = (old.id.to_string(), name.to_owned());
        for (actor, kind, scope) in &holders {
            let (actor, id) = (*actor, scope.to_string());
            let found = actors::find(self.pool, actor).await?;
            let object = format!("{kind}:{id}");
            let workspace = self.workspace_of(kind, &id).await?;
            if let (Some(workspace), true) = (workspace, found.kind == users::KIND) {
                let member = (actor, object.clone(), new_name.as_str());
                let by = caller.actor.clone();
                grants::service::join(self.pool, self.hooks, by, workspace, member).await?;
            }
            self.give(&target, (kind, *scope), actor, caller.actor.id)
                .await?;
            if let (Some(workspace), true) = (workspace, found.kind == users::KIND) {
                let member = (actor, object, old_name.as_str());
                let by = caller.actor.clone();
                grants::service::leave(self.pool, self.hooks, by, workspace, member).await?;
            }
        }

        // Delete the role with its grants.
        let mut tx = self.pool.begin().await?;
        store::delete_with_grants(&mut tx, old.id).await?;
        tx.commit().await?;
        tracing::info!(event = "role.deleted", role = %old.id, moved_to = name);
        Ok(())
    }

    /// Returns the workspace of a scope. The platform has none.
    async fn workspace_of(&self, kind: &str, id: &str) -> Result<Option<WorkspaceId>, Error> {
        let Some(search) = self
            .grants
            .get(kind)
            .and_then(|grantable| grantable.workspace(self.pool.clone(), id))
        else {
            return Ok(None);
        };
        Ok(search.await?)
    }

    /// Gives the new role to one holder on one scope.
    async fn give(
        &self,
        target: &Target,
        (kind, scope): (&str, uuid::Uuid),
        actor: ActorId,
        by: ActorId,
    ) -> Result<(), Error> {
        match target {
            Target::Relation(_, relation) => {
                let id = scope.to_string();
                let relationship = self
                    .grants
                    .get(kind)
                    .and_then(|grantable| grantable.relationship(relation, &id, actor))
                    .ok_or_else(not_a_target)?;
                self.gate.write(&[relationship]).await?;
                let mut tx = self.pool.begin().await?;
                grants::store::note_relation(&mut tx, actor, (kind, scope), relation, by).await?;
                tx.commit().await?;
            }
            Target::Row(row) => {
                let mut tx = self.pool.begin().await?;
                store::grant_row(&mut tx, actor, row.id, (kind, scope), by).await?;
                tx.commit().await?;
            }
        }
        Ok(())
    }
}

/// Returns the object of the owner of a role.
fn owner_object(workspace: Option<WorkspaceId>) -> Object {
    match workspace {
        Some(workspace) => Object::of(workspace),
        None => Object::of(Platform::ID),
    }
}
