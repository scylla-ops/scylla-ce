//! The SQL of the keys of the pipelines of a project, and of the opt-in of the Git events of an
//! editor pipeline. A file pipeline and an editor pipeline of a project share the keys.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;

/// An active editor pipeline of a project, with what gives its key.
pub(crate) struct EditorKey {
    /// The id.
    pub(crate) id: PipelineId,
    /// The display name, which gives the key.
    pub(crate) name: String,
    /// `true` when a file took its key.
    pub(crate) taken: bool,
}

/// Locks the link of a project, so a sync of its files waits, and returns `true` when an active
/// file pipeline of the default branch has a key.
pub(crate) async fn file_has_key(
    tx: &mut Tx<'_>,
    project: ProjectId,
    key: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT project_id FROM repository_links WHERE project_id = $1 FOR SHARE",
        project.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    sqlx::query_scalar!(
        r#"SELECT EXISTS (
               SELECT 1 FROM pipelines
               WHERE project_id = $1 AND source = 'file' AND file_key = $2
                 AND archived_at IS NULL AND removed_at IS NULL
           ) AS "taken!""#,
        project.to_db(),
        key,
    )
    .fetch_one(&mut **tx)
    .await
}

/// Returns the keys of the active file pipelines of the default branch of a project.
pub(crate) async fn file_keys(
    tx: &mut Tx<'_>,
    project: ProjectId,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT file_key AS "key!" FROM pipelines
           WHERE project_id = $1 AND source = 'file' AND file_key IS NOT NULL
             AND archived_at IS NULL AND removed_at IS NULL"#,
        project.to_db(),
    )
    .fetch_all(&mut **tx)
    .await
}

/// Returns the active editor pipelines of a project.
pub(crate) async fn editor_keys(
    tx: &mut Tx<'_>,
    project: ProjectId,
) -> Result<Vec<EditorKey>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, name, key_taken_at IS NOT NULL AS "taken!" FROM pipelines
           WHERE project_id = $1 AND source = 'editor' AND archived_at IS NULL"#,
        project.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| EditorKey {
            id: PipelineId::from_db(row.id),
            name: row.name,
            taken: row.taken,
        })
        .collect())
}

/// Marks pipelines whose key a file took, or frees them.
pub(crate) async fn mark_key_taken(
    tx: &mut Tx<'_>,
    ids: &[PipelineId],
    taken: bool,
) -> Result<(), sqlx::Error> {
    let ids: Vec<_> = ids.iter().map(|id| id.to_db()).collect();
    sqlx::query!(
        "UPDATE pipelines SET key_taken_at = CASE WHEN $2 THEN now() END WHERE id = ANY($1)",
        &ids,
        taken,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Sets the opt-in of the Git events of a pipeline to an actor, or removes it for `None`. An
/// opt-in that is on keeps its time and its actor.
pub(crate) async fn set_git_events(
    tx: &mut Tx<'_>,
    id: PipelineId,
    by: Option<ActorId>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE pipelines
         SET git_events_allowed_at = CASE WHEN $2::uuid IS NULL THEN NULL
                                          ELSE coalesce(git_events_allowed_at, now()) END,
             git_events_allowed_by = CASE WHEN $2::uuid IS NULL THEN NULL
                                          ELSE coalesce(git_events_allowed_by, $2) END
         WHERE id = $1",
        id.to_db(),
        by.map(ActorId::to_db),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
