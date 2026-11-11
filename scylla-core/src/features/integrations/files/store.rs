//! The SQL of the pipeline files: the source of the pipelines of a project on its link, and the
//! rows of `pipelines` that the files of the default branch give. Each function runs in the
//! transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::links::PipelineSource;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;
use serde_json::Value;

use crate::Error;

/// A file pipeline, as a file of the default branch gives it.
pub(super) struct FileRow<'a> {
    /// The key.
    pub(super) key: &'a str,
    /// The path of the file.
    pub(super) path: &'a str,
    /// The SHA of the blob, or `None` for a file that the lookup refuses.
    pub(super) blob_sha: Option<&'a str>,
    /// The display name.
    pub(super) name: &'a str,
    /// The nodes in JSON: an empty list for a file with errors.
    pub(super) nodes: Value,
    /// The rules of the `on` table in JSON, or `None` for a file with errors.
    pub(super) rules: Option<Value>,
    /// The checkout settings in JSON, or `None` for a file with errors.
    pub(super) checkout: Option<Value>,
    /// The errors in JSON, or `None` for a valid file.
    pub(super) errors: Option<Value>,
}

/// Locks the link of a project, and returns the source of its pipelines, or `None` for a
/// project with no link.
pub(super) async fn lock_source(
    tx: &mut Tx<'_>,
    project: ProjectId,
) -> Result<Option<PipelineSource>, Error> {
    let source = sqlx::query_scalar!(
        "SELECT pipeline_source FROM repository_links WHERE project_id = $1 FOR UPDATE",
        project.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    source
        .map(|source| {
            PipelineSource::parse(&source)
                .ok_or_else(|| Error::Internal(format!("unknown pipeline source {source}")))
        })
        .transpose()
}

/// Sets the source of the pipelines of a project on its link.
pub(super) async fn set_source(
    tx: &mut Tx<'_>,
    project: ProjectId,
    source: PipelineSource,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE repository_links SET pipeline_source = $2, updated_at = now()
         WHERE project_id = $1",
        project.to_db(),
        source.as_str(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Archives the active editor pipelines of a project. Returns their number.
pub(super) async fn archive_editor(
    tx: &mut Tx<'_>,
    project: ProjectId,
) -> Result<u64, sqlx::Error> {
    let done = sqlx::query!(
        "UPDATE pipelines SET archived_at = now(), updated_at = now()
         WHERE project_id = $1 AND source = 'editor' AND archived_at IS NULL",
        project.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(done.rows_affected())
}

/// Restores the archived editor pipelines of a project, and hides its file pipelines.
pub(super) async fn restore_editor(tx: &mut Tx<'_>, project: ProjectId) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE pipelines
         SET archived_at = CASE source WHEN 'editor' THEN NULL ELSE now() END, updated_at = now()
         WHERE project_id = $1
           AND ((source = 'editor' AND archived_at IS NOT NULL)
                OR (source = 'file' AND archived_at IS NULL))",
        project.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Inserts the file pipeline of a key, or changes it when its file, its errors or its state
/// changed. The pipeline becomes active and present.
pub(super) async fn upsert(
    tx: &mut Tx<'_>,
    project: ProjectId,
    actor: ActorId,
    row: &FileRow<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO pipelines (id, project_id, workspace_id, created_by, name, nodes, source,
                                file_key, file_path, file_sha, file_rules, file_errors, checkout)
         SELECT $1, p.id, p.workspace_id, $3, $4, $5, 'file', $6, $7, $8, $9, $10, $11
         FROM projects p WHERE p.id = $2
         ON CONFLICT (project_id, file_key) DO UPDATE
         SET name = excluded.name, nodes = excluded.nodes, file_path = excluded.file_path,
             file_sha = excluded.file_sha, file_rules = excluded.file_rules,
             file_errors = excluded.file_errors, checkout = excluded.checkout,
             archived_at = NULL, removed_at = NULL, version = pipelines.version + 1,
             updated_at = now()
         WHERE (pipelines.name, pipelines.file_path, pipelines.file_sha, pipelines.file_errors,
                pipelines.archived_at, pipelines.removed_at)
               IS DISTINCT FROM (excluded.name, excluded.file_path, excluded.file_sha,
                                 excluded.file_errors, NULL::timestamptz, NULL::timestamptz)",
        PipelineId::generate().to_db(),
        project.to_db(),
        actor.to_db(),
        row.name,
        row.nodes,
        row.key,
        row.path,
        row.blob_sha,
        row.rules,
        row.errors,
        row.checkout,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Marks as removed each file pipeline of a project with a key that is not in a list, and shows
/// it again. Its jobs stay.
pub(super) async fn mark_removed(
    tx: &mut Tx<'_>,
    project: ProjectId,
    keys: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE pipelines
         SET removed_at = coalesce(removed_at, now()), archived_at = NULL, updated_at = now()
         WHERE project_id = $1 AND source = 'file' AND NOT (file_key = ANY($2))
           AND (removed_at IS NULL OR archived_at IS NOT NULL)",
        project.to_db(),
        keys,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
