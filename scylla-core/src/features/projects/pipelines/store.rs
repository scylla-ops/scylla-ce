//! The SQL of the `pipelines` and `pipeline_layouts` tables. Each function runs in the transaction
//! of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::{
    NewPipeline, Pipeline, PipelineChange, PipelineId, PipelineLayout, PipelineName, PipelineScope,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;
use serde_json::Value;
use time::OffsetDateTime;

use super::json;
use crate::Error;

/// The result of a change.
pub(super) enum Changed {
    /// The pipeline with its new version.
    Done(Pipeline),
    /// The pipeline has another version.
    Stale,
    /// No pipeline has this id.
    Missing,
}

/// Inserts a pipeline in a project, with the workspace of the project. Returns `None` if no
/// project has this id.
pub(super) async fn insert(tx: &mut Tx<'_>, new: &NewPipeline) -> Result<Option<Pipeline>, Error> {
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO pipelines (id, project_id, workspace_id, created_by, name, nodes, checkout)
         SELECT $1, p.id, p.workspace_id, $3, $4, $5, $6 FROM projects p WHERE p.id = $2
         RETURNING id, project_id, workspace_id, created_by, name, nodes, checkout, version,
                   created_at, updated_at",
        PipelineId::generate().to_db(),
        new.project_id.to_db(),
        new.created_by.to_db(),
        new.name.as_str(),
        json::nodes(new.nodes.as_slice()),
        serde_json::to_value(&new.checkout).ok(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(Row::into_pipeline).transpose()
}

/// Returns a pipeline, or `None` if no pipeline has this id.
pub(crate) async fn find(tx: &mut Tx<'_>, id: PipelineId) -> Result<Option<Pipeline>, Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, project_id, workspace_id, created_by, name, nodes, checkout, version,
                created_at, updated_at
         FROM pipelines WHERE id = $1",
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
pub(super) async fn find_by_file_key(
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

/// Returns the pipelines of a project or of a workspace after the page id, in id order, with
/// one extra row.
pub(super) async fn list(
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
        "SELECT id, project_id, workspace_id, created_by, name, nodes, checkout, version,
                created_at, updated_at
         FROM pipelines
         WHERE ($1::uuid IS NULL OR project_id = $1)
           AND ($2::uuid IS NULL OR workspace_id = $2)
           AND ($3::uuid IS NULL OR id > $3)
         ORDER BY id
         LIMIT $4",
        project,
        workspace,
        page.after.map(PipelineId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_pipeline).collect()
}

/// Changes a pipeline if it still has the version of the change.
pub(super) async fn update(
    tx: &mut Tx<'_>,
    id: PipelineId,
    change: &PipelineChange,
) -> Result<Changed, Error> {
    // Change the row of this version.
    let row = sqlx::query_as!(
        Row,
        "UPDATE pipelines
         SET name = coalesce($3, name), nodes = coalesce($4, nodes),
             checkout = coalesce($5, checkout), version = version + 1, updated_at = now()
         WHERE id = $1 AND version = $2
         RETURNING id, project_id, workspace_id, created_by, name, nodes, checkout, version,
                   created_at, updated_at",
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
    if let Some(row) = row {
        return Ok(Changed::Done(row.into_pipeline()?));
    }

    // Tell a stale version from an unknown pipeline.
    Ok(match place(tx, id).await? {
        Some(_) => Changed::Stale,
        None => Changed::Missing,
    })
}

/// Deletes a pipeline with its jobs and their logs. Returns `false` if no pipeline has this id.
pub(super) async fn delete(tx: &mut Tx<'_>, id: PipelineId) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM pipelines WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// Returns the saved places of a pipeline, or `None` if nothing is saved.
pub(super) async fn find_layout(
    tx: &mut Tx<'_>,
    id: PipelineId,
) -> Result<Option<PipelineLayout>, Error> {
    let row = sqlx::query!(
        "SELECT start, nodes FROM pipeline_layouts WHERE pipeline_id = $1",
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    let layout = row
        .map(|row| json::read_layout(row.start, row.nodes))
        .transpose()?;
    Ok(layout.filter(|layout| !layout.is_empty()))
}

/// Locks a pipeline, so a change of its nodes waits, and returns the ids of its nodes. Returns
/// `None` if no pipeline has this id.
pub(super) async fn lock_node_ids(
    tx: &mut Tx<'_>,
    id: PipelineId,
) -> Result<Option<Vec<String>>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT ARRAY(SELECT node ->> 'id' FROM jsonb_array_elements(nodes) AS node)
               AS "ids!: Vec<String>"
           FROM pipelines WHERE id = $1 FOR SHARE"#,
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await
}

/// Saves places of a pipeline in one statement, so two saves at one time both stay. The other
/// places stay.
pub(super) async fn save_layout(
    tx: &mut Tx<'_>,
    id: PipelineId,
    layout: &PipelineLayout,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO pipeline_layouts (pipeline_id, start, nodes) VALUES ($1, $2, $3)
         ON CONFLICT (pipeline_id) DO UPDATE
         SET start = coalesce(excluded.start, pipeline_layouts.start),
             nodes = pipeline_layouts.nodes || excluded.nodes,
             updated_at = now()",
        id.to_db(),
        layout.start.map(json::place),
        json::places(&layout.nodes),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Removes the places of the ids that are not in a list.
pub(super) async fn keep_places(
    tx: &mut Tx<'_>,
    id: PipelineId,
    nodes: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE pipeline_layouts
         SET nodes = (SELECT coalesce(jsonb_object_agg(key, value), '{}')
                      FROM jsonb_each(nodes) WHERE key = ANY($2)),
             updated_at = now()
         WHERE pipeline_id = $1",
        id.to_db(),
        nodes,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// A row of the `pipelines` table.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The project.
    project_id: uuid::Uuid,
    /// The workspace.
    workspace_id: uuid::Uuid,
    /// The author.
    created_by: uuid::Uuid,
    /// The display name.
    name: String,
    /// The nodes in JSON.
    nodes: Value,
    /// The checkout settings in JSON, or `None` for the default.
    checkout: Option<Value>,
    /// The version.
    version: i64,
    /// The creation time.
    created_at: OffsetDateTime,
    /// The time of the last change.
    updated_at: OffsetDateTime,
}

impl Row {
    /// Makes the pipeline of the row.
    fn into_pipeline(self) -> Result<Pipeline, Error> {
        Ok(Pipeline {
            id: PipelineId::from_db(self.id),
            project_id: ProjectId::from_db(self.project_id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            created_by: ActorId::from_db(self.created_by),
            name: self.name,
            nodes: json::read_nodes(self.nodes)?,
            checkout: self
                .checkout
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| Error::Internal(format!("bad checkout of a pipeline: {error}")))?
                .unwrap_or_default(),
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}
