//! The SQL of the `projects` table. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::{
    NewProject, Project, ProjectChange, ProjectDescription, ProjectId, ProjectName, Visible,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// The foreign key from a project to its workspace.
const WORKSPACE_FOREIGN_KEY: &str = "projects_workspace_id_fkey";

/// Inserts a project and returns it. An unknown workspace gives `NotFound("workspace")`.
pub(super) async fn insert(tx: &mut Tx<'_>, new: &NewProject) -> Result<Project, Error> {
    // Insert the row. An unknown workspace breaks the foreign key of the workspace.
    let id = ProjectId::generate();
    let created_at = sqlx::query_scalar!(
        "INSERT INTO projects (id, workspace_id, created_by, name, description)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING created_at",
        id.to_db(),
        new.workspace_id.to_db(),
        new.created_by.to_db(),
        new.name.as_str(),
        new.description.as_str(),
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(WORKSPACE_FOREIGN_KEY) => {
            Error::NotFound("workspace")
        }
        error => error.into(),
    })?;

    // Return the project with the creation time of the database.
    Ok(Project {
        id,
        workspace_id: new.workspace_id,
        created_by: new.created_by,
        name: new.name.as_str().to_owned(),
        description: new.description.as_str().to_owned(),
        created_at,
    })
}

/// Returns the projects that a caller can see, of one workspace or of all, after the page id,
/// in id order, with one extra row.
pub(super) async fn list_visible(
    tx: &mut Tx<'_>,
    visible: &Visible,
    workspace: Option<WorkspaceId>,
    page: PageRequest<ProjectId>,
) -> Result<Vec<Project>, Error> {
    let workspaces: Vec<_> = visible.workspaces.iter().map(|id| id.to_db()).collect();
    let shared: Vec<_> = visible.projects.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query_as!(
        Row,
        "SELECT p.id, p.workspace_id, p.created_by, p.name, p.description, p.created_at
         FROM projects p
         WHERE ($7
                OR p.workspace_id IN (SELECT o.workspace_id FROM workspace_owners o WHERE o.user_id = $1)
                OR p.workspace_id = ANY($2)
                OR p.id = ANY($3))
           AND ($4::uuid IS NULL OR p.workspace_id = $4)
           AND ($5::uuid IS NULL OR p.id > $5)
         ORDER BY p.id
         LIMIT $6",
        visible.caller.to_db(),
        &workspaces,
        &shared,
        workspace.map(WorkspaceId::to_db),
        page.after.map(ProjectId::to_db),
        page.size.get() as i64 + 1,
        visible.all,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_project).collect())
}

/// Returns a project, or `None` if no project has this id.
pub(super) async fn find(tx: &mut Tx<'_>, id: ProjectId) -> Result<Option<Project>, Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, created_by, name, description, created_at
         FROM projects WHERE id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_project))
}

/// Changes the name or the description of a project, and returns it, or `None` if no project
/// has this id.
pub(super) async fn update(
    tx: &mut Tx<'_>,
    id: ProjectId,
    change: &ProjectChange,
) -> Result<Option<Project>, Error> {
    let row = sqlx::query_as!(
        Row,
        "UPDATE projects SET name = coalesce($2, name), description = coalesce($3, description)
         WHERE id = $1
         RETURNING id, workspace_id, created_by, name, description, created_at",
        id.to_db(),
        change.name.as_ref().map(ProjectName::as_str),
        change.description.as_ref().map(ProjectDescription::as_str),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_project))
}

/// Deletes a project with its secrets, and returns its workspace, or `None` if no project has
/// this id.
pub(super) async fn delete(tx: &mut Tx<'_>, id: ProjectId) -> Result<Option<WorkspaceId>, Error> {
    let workspace_id = sqlx::query_scalar!(
        "DELETE FROM projects WHERE id = $1 RETURNING workspace_id",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(workspace_id.map(WorkspaceId::from_db))
}

/// Returns the workspace of a project, or `None` if no project has this id.
pub(super) async fn workspace(
    tx: &mut Tx<'_>,
    id: ProjectId,
) -> Result<Option<WorkspaceId>, sqlx::Error> {
    let workspace_id = sqlx::query_scalar!(
        "SELECT workspace_id FROM projects WHERE id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(workspace_id.map(WorkspaceId::from_db))
}

/// A row of the `projects` table.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The workspace.
    workspace_id: uuid::Uuid,
    /// The author.
    created_by: uuid::Uuid,
    /// The display name.
    name: String,
    /// The description, empty for none.
    description: String,
    /// The creation time.
    created_at: OffsetDateTime,
}

impl Row {
    /// Makes the project of the row.
    fn into_project(self) -> Project {
        Project {
            id: ProjectId::from_db(self.id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            created_by: ActorId::from_db(self.created_by),
            name: self.name,
            description: self.description,
            created_at: self.created_at,
        }
    }
}
