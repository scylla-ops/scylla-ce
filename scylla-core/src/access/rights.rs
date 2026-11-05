//! The rights of an actor on an object: the instance admins, the owners of the workspace and the
//! roles that are rows from Postgres, then the roles of the schema from SpiceDB.

use std::collections::HashSet;

use scylla_domain::authz::{Check, Object, Resource, Schema};
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::catalog;
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::AccessError;
use sqlx::PgPool;

use super::{store, SpiceDb};

/// Finds the rights of an actor: the instance admins, the owners of the workspaces and the roles
/// that are rows in Postgres, and the roles of the schema in SpiceDB.
pub(super) struct Rights {
    /// The database pool, for the instance admins, the owners and the roles that are rows.
    pub(super) pool: PgPool,
    /// The SpiceDB client, for the roles.
    pub(super) spicedb: SpiceDb,
    /// The permission model: it tells which definitions have a permission.
    pub(super) schema: Schema,
}

impl Rights {
    /// Returns `true` if the caller is the object itself, an instance admin, an owner of the
    /// workspace of the object, or has a role that gives the permission on the object, on its
    /// workspace or on the instance.
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
        let workspace = workspace_of(object, workspace);
        if self.passes_all(caller, object, workspace).await? {
            return Ok(vec![true; permissions.len()]);
        }

        // Pass the permissions that a role that is a row gives.
        let held = self.held_in_rows(caller, object, workspace).await?;
        let given: Vec<bool> = permissions
            .iter()
            .map(|permission| {
                catalog::covering(object.kind(), permission)
                    .iter()
                    .any(|id| held.contains(*id))
            })
            .collect();
        if given.iter().all(|given| *given) {
            return Ok(given);
        }

        // Ask for each other permission on the object and on its workspace, where the schema has it.
        let container = workspace
            .filter(|_| object.kind() != Workspace::NAME)
            .map(Object::of);
        let mut items = Vec::new();
        let mut asked = Vec::new();
        for (index, &permission) in permissions.iter().enumerate() {
            if given[index] {
                continue;
            }
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
            return Ok(given);
        }
        let subject = Object::of(caller);
        let replies = self.spicedb.check_bulk(&subject, &items, fresh).await?;

        // Pass a permission on one yes. With no yes, an error on one of its items fails closed.
        let mut allowed: Vec<_> = given.into_iter().map(Some).collect();
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

    /// Returns, for each permission of the catalog, `true` if the caller holds it on an object: it
    /// passes each permission with no role, a role that is a row has it, or the roles of the
    /// schema give each of its routes on the object.
    pub(super) async fn holds(
        &self,
        caller: ActorId,
        object: &Object,
        workspace: Option<WorkspaceId>,
        permissions: &[&'static str],
    ) -> Result<Vec<bool>, AccessError> {
        // Pass each permission with no role, then read the roles that are rows.
        let workspace = workspace_of(object, workspace);
        if self.passes_all(caller, object, workspace).await? {
            return Ok(vec![true; permissions.len()]);
        }
        let held = self.held_in_rows(caller, object, workspace).await?;

        // Ask the schema for the routes of each other permission.
        let mut answers = Vec::with_capacity(permissions.len());
        for id in permissions {
            let routes: Vec<&'static str> = catalog::find(id)
                .map(|(_, permission)| permission.routes)
                .unwrap_or_default()
                .iter()
                .map(|route| route.permission)
                .filter(|route| self.schema.has(object.kind(), route))
                .collect();
            let mut holds = held.contains(*id);
            if !holds && !routes.is_empty() {
                let replies = self
                    .answers(caller, object, workspace, &routes, None)
                    .await?;
                holds = replies.into_iter().all(|reply| reply);
            }
            answers.push(holds);
        }
        Ok(answers)
    }

    /// Returns the ids of the objects of a definition where a role that is a row gives a route
    /// permission to the actor.
    pub(super) async fn row_objects(
        &self,
        actor: ActorId,
        definition: &'static str,
        permission: &'static str,
    ) -> Result<Vec<String>, AccessError> {
        let ids = catalog::covering(definition, permission);
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut tx = self.pool.begin().await.map_err(store::failed)?;
        let objects = store::objects_in_rows(&mut tx, actor, definition, ids)
            .await
            .map_err(store::failed)?;
        tx.commit().await.map_err(store::failed)?;
        Ok(objects)
    }

    /// Returns the ids of the permissions that the roles that are rows give to the actor on an
    /// object: on the object, on its workspace and on the instance. A route on an actor has none.
    async fn held_in_rows(
        &self,
        caller: ActorId,
        object: &Object,
        workspace: Option<WorkspaceId>,
    ) -> Result<HashSet<String>, AccessError> {
        if object.kind() == Actor::NAME {
            return Ok(HashSet::new());
        }
        let project = (object.kind() == Project::NAME)
            .then(|| ProjectId::parse(object.id()).ok())
            .flatten();
        let mut tx = self.pool.begin().await.map_err(store::failed)?;
        let held = store::held_in_rows(&mut tx, caller, workspace, project)
            .await
            .map_err(store::failed)?;
        tx.commit().await.map_err(store::failed)?;
        Ok(held.into_iter().collect())
    }

    /// Returns `true` if the caller has each permission with no role: it is the object itself,
    /// an instance admin, or an owner of the workspace.
    async fn passes_all(
        &self,
        caller: ActorId,
        object: &Object,
        workspace: Option<WorkspaceId>,
    ) -> Result<bool, AccessError> {
        // Pass the actor itself, and an instance admin on each object.
        if object.kind() == Actor::NAME && ActorId::parse(object.id()).ok() == Some(caller) {
            return Ok(true);
        }
        if self.is_admin(caller).await? {
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

/// Returns the workspace of an object: the object itself for a workspace.
fn workspace_of(object: &Object, workspace: Option<WorkspaceId>) -> Option<WorkspaceId> {
    if object.kind() == Workspace::NAME {
        WorkspaceId::parse(object.id()).ok()
    } else {
        workspace
    }
}
