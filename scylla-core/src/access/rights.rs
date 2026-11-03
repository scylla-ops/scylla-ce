//! The rights of an actor on an object: the instance admins and the owners of the workspace
//! from Postgres, then the roles from SpiceDB.

use scylla_domain::authz::{Check, Object, Resource, Schema};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::AccessError;
use sqlx::PgPool;

use super::{store, SpiceDb};

/// Finds the rights of an actor: the instance admins and the owners of the workspaces in
/// Postgres, and the roles in SpiceDB.
pub(super) struct Rights {
    /// The database pool, for the instance admins and the owners of the workspaces.
    pub(super) pool: PgPool,
    /// The SpiceDB client, for the roles.
    pub(super) spicedb: SpiceDb,
    /// The permission model: it tells which definitions have a permission.
    pub(super) schema: Schema,
}

impl Rights {
    /// Returns `true` if the caller is the object itself, an instance admin on the platform, an
    /// owner of the workspace of the object, or has a role that gives the permission on the
    /// object or on its workspace.
    pub(super) async fn allow(&self, caller: ActorId, check: &Check) -> Result<bool, AccessError> {
        let (object, workspace) = (check.object(), check.workspace());
        let permissions = [check.permission()];
        let answers = self
            .answers(caller, object, workspace, &permissions, check.freshness())
            .await?;
        Ok(answers.first().copied().unwrap_or(false))
    }

    /// Returns, for each permission, `true` if the caller has it on the object, as `allow`
    /// finds it. One SpiceDB call asks for all the permissions that the fast paths do not give.
    pub(super) async fn answers(
        &self,
        caller: ActorId,
        object: &Object,
        workspace: Option<WorkspaceId>,
        permissions: &[&'static str],
        fresh: Option<&str>,
    ) -> Result<Vec<bool>, AccessError> {
        // Pass each permission for the actor itself, an instance admin or an owner.
        let workspace = if object.kind() == Workspace::NAME {
            WorkspaceId::parse(object.id()).ok()
        } else {
            workspace
        };
        if self.passes_all(caller, object, workspace).await? {
            return Ok(vec![true; permissions.len()]);
        }

        // Ask for each permission on the object and on its workspace, where the schema has it.
        let container = workspace
            .filter(|_| object.kind() != Workspace::NAME)
            .map(Object::of);
        let mut items = Vec::new();
        let mut asked = Vec::new();
        for (index, &permission) in permissions.iter().enumerate() {
            if self.schema.has(object.kind(), permission) {
                items.push((object.clone(), permission));
                asked.push(index);
            }
            if let Some(container) = container.as_ref() {
                if self.schema.has(Workspace::NAME, permission) {
                    items.push((container.clone(), permission));
                    asked.push(index);
                }
            }
        }
        if items.is_empty() {
            return Ok(vec![false; permissions.len()]);
        }
        let subject = Object::of(caller);
        let replies = self.spicedb.check_bulk(&subject, &items, fresh).await?;

        // Pass a permission on one yes. With no yes, an error on one of its items fails closed.
        let mut allowed = vec![Some(false); permissions.len()];
        for (index, reply) in asked.into_iter().zip(replies) {
            allowed[index] = match (allowed[index], reply) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (None, _) | (_, None) => None,
                _ => Some(false),
            };
        }
        allowed
            .into_iter()
            .map(|answer| {
                answer.ok_or_else(|| {
                    AccessError::Unavailable("permission system unavailable".to_owned())
                })
            })
            .collect()
    }

    /// Returns `true` if the caller has each permission with no role: it is the object itself,
    /// an instance admin on the platform, or an owner of the workspace.
    async fn passes_all(
        &self,
        caller: ActorId,
        object: &Object,
        workspace: Option<WorkspaceId>,
    ) -> Result<bool, AccessError> {
        // Pass the actor itself, and an instance admin on the platform.
        if object.kind() == Actor::NAME && ActorId::parse(object.id()).ok() == Some(caller) {
            return Ok(true);
        }
        if object.kind() == Platform::NAME && self.is_admin(caller).await? {
            return Ok(true);
        }

        // Pass an owner of the workspace.
        match workspace {
            Some(workspace) => self.owns(caller, workspace).await,
            None => Ok(false),
        }
    }

    /// Returns `true` if the caller is one of the owners of a workspace.
    async fn owns(&self, caller: ActorId, workspace: WorkspaceId) -> Result<bool, AccessError> {
        let mut tx = self.pool.begin().await.map_err(store::failed)?;
        let owns = store::is_owner(&mut tx, workspace, caller)
            .await
            .map_err(store::failed)?;
        tx.commit().await.map_err(store::failed)?;
        Ok(owns)
    }

    /// Returns `true` if the caller is an admin of the instance.
    async fn is_admin(&self, caller: ActorId) -> Result<bool, AccessError> {
        let mut tx = self.pool.begin().await.map_err(store::failed)?;
        let admin = store::is_admin(&mut tx, caller)
            .await
            .map_err(store::failed)?;
        tx.commit().await.map_err(store::failed)?;
        Ok(admin)
    }
}
