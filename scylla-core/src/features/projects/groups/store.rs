//! The SQL of the `project_groups` table and of the places of the projects. Each function runs in
//! the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::groups::{NewProjectGroup, ProjectGroup, ProjectGroupId};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// The foreign key from a group to its workspace.
const WORKSPACE_FOREIGN_KEY: &str = "project_groups_workspace_id_fkey";

/// Locks the order of the groups and of the projects of a workspace until the end of the
/// transaction. A new project in the workspace does not wait for it.
pub(super) async fn lock_workspace(tx: &mut Tx<'_>, workspace: WorkspaceId) -> Result<(), Error> {
    sqlx::query!(
        "SELECT id FROM workspaces WHERE id = $1 FOR NO KEY UPDATE",
        workspace.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(())
}

/// Inserts a group after the other groups of its workspace, and returns it. An unknown workspace
/// gives `NotFound("workspace")`.
pub(super) async fn insert(tx: &mut Tx<'_>, new: &NewProjectGroup) -> Result<ProjectGroup, Error> {
    let id = ProjectGroupId::generate();
    let created_at = sqlx::query_scalar!(
        "INSERT INTO project_groups (id, workspace_id, created_by, name, position)
         VALUES ($1, $2, $3, $4,
                 (SELECT coalesce(max(position) + 1, 0) FROM project_groups
                  WHERE workspace_id = $2))
         RETURNING created_at",
        id.to_db(),
        new.workspace_id.to_db(),
        new.created_by.to_db(),
        new.name.as_str(),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(WORKSPACE_FOREIGN_KEY) => {
            Error::NotFound("workspace")
        }
        error => error.into(),
    })?;
    Ok(ProjectGroup {
        id,
        workspace_id: new.workspace_id,
        created_by: new.created_by,
        name: new.name.as_str().to_owned(),
        created_at,
    })
}

/// Returns the groups of a workspace in their order.
pub(super) async fn list(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<Vec<ProjectGroup>, Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, created_by, name, created_at FROM project_groups
         WHERE workspace_id = $1 ORDER BY position, id",
        workspace.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_group).collect())
}

/// Returns a group, or `None` if no group has this id.
pub(super) async fn find(
    tx: &mut Tx<'_>,
    id: ProjectGroupId,
) -> Result<Option<ProjectGroup>, Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, created_by, name, created_at FROM project_groups WHERE id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_group))
}

/// Returns the workspace of a group, or `None` if no group has this id.
pub(super) async fn workspace(
    tx: &mut Tx<'_>,
    id: ProjectGroupId,
) -> Result<Option<WorkspaceId>, Error> {
    let workspace = sqlx::query_scalar!(
        "SELECT workspace_id FROM project_groups WHERE id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(workspace.map(WorkspaceId::from_db))
}

/// Gives a group a new name.
pub(super) async fn rename(tx: &mut Tx<'_>, id: ProjectGroupId, name: &str) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE project_groups SET name = $2 WHERE id = $1",
        id.to_db(),
        name
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Gives the groups their places: the first id gets the place 0.
pub(super) async fn set_order(tx: &mut Tx<'_>, ids: &[ProjectGroupId]) -> Result<(), Error> {
    let ids: Vec<_> = ids.iter().map(|id| id.to_db()).collect();
    sqlx::query!(
        "UPDATE project_groups g SET position = (o.n - 1)::integer
         FROM unnest($1::uuid[]) WITH ORDINALITY AS o(id, n)
         WHERE g.id = o.id",
        &ids
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes a group.
pub(super) async fn delete(tx: &mut Tx<'_>, id: ProjectGroupId) -> Result<(), Error> {
    sqlx::query!("DELETE FROM project_groups WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Returns the projects of a group of a workspace, or of no group, in their order: the places,
/// then the projects with no place, oldest first.
pub(super) async fn section(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    group: Option<ProjectGroupId>,
) -> Result<Vec<ProjectId>, Error> {
    let ids = sqlx::query_scalar!(
        "SELECT id FROM projects
         WHERE workspace_id = $1 AND group_id IS NOT DISTINCT FROM $2::uuid
         ORDER BY position NULLS LAST, id",
        workspace.to_db(),
        group.map(ProjectGroupId::to_db),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(ids.into_iter().map(ProjectId::from_db).collect())
}

/// Puts projects in a group, or in no group, at their places: the first id gets the place 0.
pub(super) async fn set_section(
    tx: &mut Tx<'_>,
    group: Option<ProjectGroupId>,
    ids: &[ProjectId],
) -> Result<(), Error> {
    let ids: Vec<_> = ids.iter().map(|id| id.to_db()).collect();
    sqlx::query!(
        "UPDATE projects p SET group_id = $2, position = (o.n - 1)::integer
         FROM unnest($1::uuid[]) WITH ORDINALITY AS o(id, n)
         WHERE p.id = o.id",
        &ids,
        group.map(ProjectGroupId::to_db),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A row of the `project_groups` table.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The workspace.
    workspace_id: uuid::Uuid,
    /// The author.
    created_by: uuid::Uuid,
    /// The display name.
    name: String,
    /// The creation time.
    created_at: OffsetDateTime,
}

impl Row {
    /// Makes the group of the row.
    fn into_group(self) -> ProjectGroup {
        ProjectGroup {
            id: ProjectGroupId::from_db(self.id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            created_by: ActorId::from_db(self.created_by),
            name: self.name,
            created_at: self.created_at,
        }
    }
}
