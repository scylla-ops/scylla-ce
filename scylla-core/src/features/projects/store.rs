//! The SQL of the `projects` table. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::images::{Icon, ImageId};
use scylla_domain::projects::groups::ProjectGroupId;
use scylla_domain::projects::lists::ListedProject;
use scylla_domain::projects::{
    NewProject, Project, ProjectChange, ProjectDescription, ProjectId, ProjectName, Visible,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
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
        icon: None,
        group_id: None,
    })
}

/// Returns each project that a caller can see, of one workspace or of all, with the facts of
/// the list: its repository, the names of its pipelines, the number of its failing pipelines,
/// its jobs that run or wait, its newest job, and its places. A failing pipeline has a last ended
/// job, of the default branch of the linked repository, that did not complete. An archived
/// pipeline from a file does not count, as in the summaries of the pipelines.
pub(super) async fn list_visible(
    tx: &mut Tx<'_>,
    visible: &Visible,
    workspace: Option<WorkspaceId>,
) -> Result<Vec<ListedProject>, Error> {
    let workspaces: Vec<_> = visible.workspaces.iter().map(|id| id.to_db()).collect();
    let shared: Vec<_> = visible.projects.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"WITH active AS (
               SELECT DISTINCT project_id FROM jobs
               WHERE status IN ('awaiting_approval', 'pending', 'running')
                 AND ($4::uuid IS NULL OR workspace_id = $4)
           )
           SELECT p.id AS "id!", p.workspace_id AS "workspace_id!",
                  p.created_by AS "created_by!", p.name AS "name!",
                  p.description AS "description!", p.created_at AS "created_at!",
                  p.icon_name AS "icon_name?", p.icon_color AS "icon_color?",
                  p.icon_image_id AS "icon_image_id?", p.group_id AS "group_id?",
                  p.position AS "position?", g.position AS "group_position?",
                  l.full_name AS "repository?",
                  coalesce(f.names, '{}') AS "pipelines!: Vec<String>",
                  coalesce(f.failing, 0) AS "failing!",
                  p.id IN (SELECT project_id FROM active) AS "running!",
                  newest.created_at AS "last_job_at?"
           FROM projects p
           LEFT JOIN project_groups g ON g.id = p.group_id
           LEFT JOIN repository_links l ON l.project_id = p.id
           LEFT JOIN LATERAL (
               SELECT array_agg(pl.name ORDER BY pl.id) AS names,
                      count(*) FILTER (WHERE last.status <> 'completed') AS failing
               FROM pipelines pl
               LEFT JOIN LATERAL (
                   SELECT j.status FROM jobs j
                   LEFT JOIN job_sources s ON s.job_id = j.id
                   WHERE j.pipeline_id = pl.id AND j.finished_at IS NOT NULL
                     AND (l.project_id IS NULL OR s.branch = l.default_branch)
                   ORDER BY j.id DESC LIMIT 1
               ) last ON true
               WHERE pl.project_id = p.id AND (pl.source = 'editor' OR pl.archived_at IS NULL)
           ) f ON true
           LEFT JOIN LATERAL (
               SELECT j.created_at FROM jobs j WHERE j.project_id = p.id
               ORDER BY j.created_at DESC LIMIT 1
           ) newest ON true
           WHERE ($5
                  OR p.workspace_id IN (SELECT o.workspace_id FROM workspace_owners o
                                        WHERE o.user_id = $1)
                  OR p.workspace_id = ANY($2)
                  OR p.id = ANY($3))
             AND ($4::uuid IS NULL OR p.workspace_id = $4)"#,
        visible.caller.to_db(),
        &workspaces,
        &shared,
        workspace.map(WorkspaceId::to_db),
        visible.all,
    )
    .fetch_all(&mut **tx)
    .await?;
    let listed = rows.into_iter().map(|row| ListedProject {
        project: Row {
            id: row.id,
            workspace_id: row.workspace_id,
            created_by: row.created_by,
            name: row.name,
            description: row.description,
            created_at: row.created_at,
            icon_name: row.icon_name,
            icon_color: row.icon_color,
            icon_image_id: row.icon_image_id,
            group_id: row.group_id,
        }
        .into_project(),
        repository: row.repository,
        pipelines: row.pipelines,
        failing: u32::try_from(row.failing).unwrap_or(u32::MAX),
        running: row.running,
        last_job_at: row.last_job_at,
        position: row.position,
        group_position: row.group_position,
    });
    Ok(listed.collect())
}

/// Returns a project, or `None` if no project has this id.
pub(super) async fn find(tx: &mut Tx<'_>, id: ProjectId) -> Result<Option<Project>, Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, created_by, name, description, created_at, icon_name,
                icon_color, icon_image_id, group_id
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
         RETURNING id, workspace_id, created_by, name, description, created_at, icon_name,
                   icon_color, icon_image_id, group_id",
        id.to_db(),
        change.name.as_ref().map(ProjectName::as_str),
        change.description.as_ref().map(ProjectDescription::as_str),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_project))
}

/// Deletes a project with its secrets, and returns its workspace and the image of its icon, or
/// `None` if no project has this id.
pub(super) async fn delete(
    tx: &mut Tx<'_>,
    id: ProjectId,
) -> Result<Option<(WorkspaceId, Option<ImageId>)>, Error> {
    let row = sqlx::query!(
        "DELETE FROM projects WHERE id = $1 RETURNING workspace_id, icon_image_id",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| {
        let image = row.icon_image_id.map(ImageId::from_db);
        (WorkspaceId::from_db(row.workspace_id), image)
    }))
}

/// Locks the row of a project and returns the image of its icon, or `None` if no project has
/// this id.
pub(super) async fn lock_icon(
    tx: &mut Tx<'_>,
    id: ProjectId,
) -> Result<Option<Option<ImageId>>, Error> {
    let image = sqlx::query_scalar!(
        "SELECT icon_image_id FROM projects WHERE id = $1 FOR UPDATE",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(image.map(|image| image.map(ImageId::from_db)))
}

/// Sets the custom icon of a project. `None` gives the default icon.
pub(super) async fn set_icon(
    tx: &mut Tx<'_>,
    id: ProjectId,
    icon: Option<&Icon>,
) -> Result<(), Error> {
    let (name, color, image) = match icon {
        None => (None, None, None),
        Some(Icon::Glyph(glyph)) => (
            Some(glyph.name.as_str()),
            glyph.color.as_ref().map(|color| color.as_str()),
            None,
        ),
        Some(Icon::Image(image)) => (None, None, Some(image.to_db())),
    };
    sqlx::query!(
        "UPDATE projects SET icon_name = $2, icon_color = $3, icon_image_id = $4 WHERE id = $1",
        id.to_db(),
        name,
        color,
        image,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
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
    /// The name of the glyph of the icon, if it has one.
    icon_name: Option<String>,
    /// The color of the glyph of the icon, if it has one.
    icon_color: Option<String>,
    /// The image of the icon, if it has one.
    icon_image_id: Option<uuid::Uuid>,
    /// The group, if the project has one.
    group_id: Option<uuid::Uuid>,
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
            icon: Icon::of_columns(
                self.icon_name,
                self.icon_color,
                self.icon_image_id.map(ImageId::from_db),
            ),
            group_id: self.group_id.map(ProjectGroupId::from_db),
        }
    }
}
