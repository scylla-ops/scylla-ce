//! The SQL of the `pipelines` table.

use scylla_domain::projects::pipelines::{
    NewPipeline, Pipeline, PipelineChange, PipelineId, PipelineName, PipelineScope,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;

use super::super::json;
use super::row::Row;
use crate::Error;

/// The result of a change.
pub(crate) enum Changed {
    /// The pipeline with its new version.
    Done(Box<Pipeline>),
    /// The pipeline has another version.
    Stale,
    /// No pipeline has this id.
    Missing,
}

/// Inserts a pipeline in a project, with the workspace of the project. Returns `None` if no
/// project has this id.
pub(crate) async fn insert(tx: &mut Tx<'_>, new: &NewPipeline) -> Result<Option<Pipeline>, Error> {
    let id = sqlx::query_scalar!(
        "INSERT INTO pipelines (id, project_id, workspace_id, created_by, name, nodes, checkout)
         SELECT $1, p.id, p.workspace_id, $3, $4, $5, $6 FROM projects p WHERE p.id = $2
         RETURNING id",
        PipelineId::generate().to_db(),
        new.project_id.to_db(),
        new.created_by.to_db(),
        new.name.as_str(),
        json::nodes(new.nodes.as_slice()),
        serde_json::to_value(&new.checkout).ok(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    match id {
        Some(id) => find(tx, PipelineId::from_db(id)).await,
        None => Ok(None),
    }
}

/// Returns a pipeline, or `None` if no pipeline has this id.
pub(crate) async fn find(tx: &mut Tx<'_>, id: PipelineId) -> Result<Option<Pipeline>, Error> {
    let row = sqlx::query_as!(
        Row,
        r#"SELECT id, project_id, workspace_id, created_by, name, nodes, checkout, version,
                  created_at, updated_at, source, file_key, file_path,
                  jsonb_array_length(coalesce(file_errors, '[]')) AS "file_errors!", archived_at,
                  removed_at
           FROM pipelines WHERE id = $1"#,
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(Row::into_pipeline).transpose()
}

/// Returns the project and the workspace of a pipeline, or `None` if no pipeline has this id.
pub(crate) async fn place(
    tx: &mut Tx<'_>,
    id: PipelineId,
) -> Result<Option<(ProjectId, WorkspaceId)>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT project_id, workspace_id FROM pipelines WHERE id = $1",
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| {
        (
            ProjectId::from_db(row.project_id),
            WorkspaceId::from_db(row.workspace_id),
        )
    }))
}

/// Returns the pipeline of a project with a file key, or `None` if the project has none.
pub(crate) async fn find_by_file_key(
    tx: &mut Tx<'_>,
    project: ProjectId,
    key: &str,
) -> Result<Option<PipelineId>, sqlx::Error> {
    let id = sqlx::query_scalar!(
        "SELECT id FROM pipelines WHERE project_id = $1 AND file_key = $2",
        project.to_db(),
        key,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(id.map(PipelineId::from_db))
}

/// Returns `true` when a project takes its pipelines from the files of its repository. The link
/// stays locked, so a change of the source waits.
pub(crate) async fn uses_files(tx: &mut Tx<'_>, project: ProjectId) -> Result<bool, sqlx::Error> {
    let source = sqlx::query_scalar!(
        "SELECT pipeline_source FROM repository_links WHERE project_id = $1 FOR SHARE",
        project.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(source.as_deref() == Some("file"))
}

/// Returns the pipelines of a project or of a workspace after the page id, in id order, with
/// one extra row. A file pipeline of a project that moved back to the editor is hidden.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    scope: PipelineScope,
    page: PageRequest<PipelineId>,
) -> Result<Vec<Pipeline>, Error> {
    let (project, workspace) = match scope {
        PipelineScope::Project(id) => (Some(id.to_db()), None),
        PipelineScope::Workspace(id) => (None, Some(id.to_db())),
    };
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT id, project_id, workspace_id, created_by, name, nodes, checkout, version,
                  created_at, updated_at, source, file_key, file_path,
                  jsonb_array_length(coalesce(file_errors, '[]')) AS "file_errors!", archived_at,
                  removed_at
           FROM pipelines
           WHERE ($1::uuid IS NULL OR project_id = $1)
             AND ($2::uuid IS NULL OR workspace_id = $2)
             AND ($3::uuid IS NULL OR id > $3)
             AND (source = 'editor' OR archived_at IS NULL)
           ORDER BY id
           LIMIT $4"#,
        project,
        workspace,
        page.after.map(PipelineId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_pipeline).collect()
}

/// Locks a pipeline, and returns `true` when it is read only: a file pipeline, or an archived
/// pipeline. Returns `None` if no pipeline has this id.
pub(crate) async fn lock_read_only(
    tx: &mut Tx<'_>,
    id: PipelineId,
) -> Result<Option<bool>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT source = 'file' OR archived_at IS NOT NULL AS "read_only!"
           FROM pipelines WHERE id = $1 FOR UPDATE"#,
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await
}

/// Changes a pipeline if it still has the version of the change.
pub(crate) async fn update(
    tx: &mut Tx<'_>,
    id: PipelineId,
    change: &PipelineChange,
) -> Result<Changed, Error> {
    // Change the row of this version.
    let changed = sqlx::query_scalar!(
        "UPDATE pipelines
         SET name = coalesce($3, name), nodes = coalesce($4, nodes),
             checkout = coalesce($5, checkout), version = version + 1, updated_at = now()
         WHERE id = $1 AND version = $2
         RETURNING id",
        id.to_db(),
        change.version,
        change.name.as_ref().map(PipelineName::as_str),
        change
            .nodes
            .as_ref()
            .map(|nodes| json::nodes(nodes.as_slice())),
        change
            .checkout
            .as_ref()
            .and_then(|new| serde_json::to_value(new).ok()),
    )
    .fetch_optional(&mut **tx)
    .await?;
    if changed.is_some() {
        let pipeline = find(tx, id).await?.ok_or(Error::NotFound("pipeline"))?;
        return Ok(Changed::Done(Box::new(pipeline)));
    }

    // Tell a stale version from an unknown pipeline.
    Ok(match place(tx, id).await? {
        Some(_) => Changed::Stale,
        None => Changed::Missing,
    })
}

/// Deletes a pipeline with its jobs and their logs. Returns `false` if no pipeline has this id.
pub(crate) async fn delete(tx: &mut Tx<'_>, id: PipelineId) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM pipelines WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected() == 1)
}
