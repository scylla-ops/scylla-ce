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
        // Pass an actor that acts on itself.
        let object = check.object();
        if object.kind() == Actor::NAME && ActorId::parse(object.id()).ok() == Some(caller) {
            return Ok(true);
        }

        // Pass an instance admin on the platform.
        if object.kind() == Platform::NAME && self.is_admin(caller).await? {
            return Ok(true);
        }

        // Pass an owner of the workspace: the object itself, or the workspace of the check.
        let workspace = if object.kind() == Workspace::NAME {
            WorkspaceId::parse(object.id()).ok()
        } else {
            check.workspace()
        };
        if let Some(workspace) = workspace {
            if self.owns(caller, workspace).await? {
                return Ok(true);
            }
        }

        // Ask SpiceDB for a role on the object or on its workspace, in one call.
        let permission = check.permission();
        let mut objects = Vec::new();
        if self.schema.has(object.kind(), permission) {
            objects.push(object.clone());
        }
        let container = workspace.filter(|_| object.kind() != Workspace::NAME);
        if let Some(workspace) = container.filter(|_| self.schema.has(Workspace::NAME, permission))
        {
            objects.push(Object::of(workspace));
        }
        if objects.is_empty() {
            return Ok(false);
        }
        let subject = Object::of(caller);
        self.spicedb
            .check_any(&subject, &objects, permission, check.freshness())
            .await
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
