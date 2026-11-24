//! The SQL of the merge blocks: the rules of a project with their pipelines, the lock of the
//! link with its ruleset, and the writes of a change. Each function runs in the transaction of
//! the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::merge_blocks::{MergeBlock, MergeBlockState};
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// A pipeline of a project, with what names its report.
pub(super) struct Pipeline {
    /// The id.
    pub(super) id: PipelineId,
    /// The name.
    pub(super) name: String,
    /// The key of a file pipeline, or `None` for a pipeline of the editor.
    pub(super) file_key: Option<String>,
}

/// A pipeline of a project, and its rule or `None`.
pub(super) struct PipelineRule {
    /// The pipeline.
    pub(super) pipeline: Pipeline,
    /// The rule of the pipeline, or `None` when it has no rule.
    pub(super) block: Option<MergeBlock>,
}

/// A change of a rule by a person.
pub(super) struct Change<'a> {
    /// The project.
    pub(super) project: ProjectId,
    /// The pipeline, or `None` for the rule of the project on GitLab.
    pub(super) pipeline: Option<PipelineId>,
    /// The name of the report of the pipeline, or `None` for the rule of the project.
    pub(super) context: Option<&'a str>,
    /// `true` when the rule must be on.
    pub(super) enabled: bool,
    /// The actor of the change.
    pub(super) actor: ActorId,
}

/// The columns of a row of `merge_blocks`.
struct Columns {
    /// The pipeline, or `None`.
    pipeline: Option<Uuid>,
    /// The name of the report, or `None`.
    context: Option<String>,
    /// `true` when the rule must be on.
    enabled: bool,
    /// The state.
    state: String,
    /// The reason of the state.
    state_reason: Option<String>,
    /// The actor of the last change.
    updated_by: Uuid,
    /// The time of the last change.
    updated_at: OffsetDateTime,
}

/// Returns the pipelines of a project in id order, each with its rule. A removed or archived
/// pipeline is in the list only when its rule must be on.
pub(super) async fn pipeline_rules(
    tx: &mut Tx<'_>,
    project: ProjectId,
) -> Result<Vec<PipelineRule>, Error> {
    let rows = sqlx::query!(
        r#"SELECT p.id, p.name, p.file_key, b.context AS "context?", b.enabled AS "enabled?",
                  b.state AS "state?", b.state_reason AS "state_reason?",
                  b.updated_by AS "updated_by?", b.updated_at AS "updated_at?"
           FROM pipelines p
           LEFT JOIN merge_blocks b ON b.project_id = p.project_id AND b.pipeline_id = p.id
           WHERE p.project_id = $1
             AND ((p.removed_at IS NULL AND p.archived_at IS NULL) OR b.enabled)
           ORDER BY p.id"#,
        project.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            let columns = match (row.enabled, row.state, row.updated_by, row.updated_at) {
                (Some(enabled), Some(state), Some(updated_by), Some(updated_at)) => Some(Columns {
                    pipeline: Some(row.id),
                    context: row.context,
                    enabled,
                    state,
                    state_reason: row.state_reason,
                    updated_by,
                    updated_at,
                }),
                _ => None,
            };
            Ok(PipelineRule {
                pipeline: Pipeline {
                    id: PipelineId::from_db(row.id),
                    name: row.name,
                    file_key: row.file_key,
                },
                block: columns.map(|columns| block(project, columns)).transpose()?,
            })
        })
        .collect()
}

/// Returns the rule of a pipeline, or the rule of the project for `None`.
pub(super) async fn find(
    tx: &mut Tx<'_>,
    project: ProjectId,
    pipeline: Option<PipelineId>,
) -> Result<Option<MergeBlock>, Error> {
    let row = sqlx::query_as!(
        Columns,
        r#"SELECT pipeline_id AS pipeline, context, enabled, state, state_reason, updated_by,
                  updated_at
           FROM merge_blocks
           WHERE project_id = $1 AND pipeline_id IS NOT DISTINCT FROM $2"#,
        project.to_db(),
        pipeline.map(PipelineId::to_db),
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(|columns| block(project, columns)).transpose()
}

/// Returns a pipeline of a project, or `None` for a pipeline of another project.
pub(super) async fn pipeline(
    tx: &mut Tx<'_>,
    project: ProjectId,
    id: PipelineId,
) -> Result<Option<Pipeline>, Error> {
    let row = sqlx::query!(
        "SELECT name, file_key FROM pipelines WHERE id = $1 AND project_id = $2",
        id.to_db(),
        project.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Pipeline {
        id,
        name: row.name,
        file_key: row.file_key,
    }))
}

/// Locks the link of a project, and returns the id of its GitHub ruleset. `None` for a project
/// with no link.
pub(super) async fn lock_link(
    tx: &mut Tx<'_>,
    project: ProjectId,
) -> Result<Option<Option<i64>>, Error> {
    let query = sqlx::query_scalar!(
        "SELECT ruleset_id FROM repository_links WHERE project_id = $1 FOR UPDATE",
        project.to_db(),
    );
    Ok(query.fetch_optional(&mut **tx).await?)
}

/// Returns the id of the GitHub ruleset of the link of a project, or `None`.
pub(super) async fn ruleset(tx: &mut Tx<'_>, project: ProjectId) -> Result<Option<i64>, Error> {
    let query = sqlx::query_scalar!(
        "SELECT ruleset_id FROM repository_links WHERE project_id = $1",
        project.to_db(),
    );
    Ok(query.fetch_optional(&mut **tx).await?.flatten())
}

/// Writes the wanted value of a rule, with the state `applying`.
pub(super) async fn upsert(tx: &mut Tx<'_>, change: &Change<'_>) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO merge_blocks (project_id, pipeline_id, context, enabled, state, updated_by)
         VALUES ($1, $2, $3, $4, 'applying', $5)
         ON CONFLICT (project_id, pipeline_id) DO UPDATE
         SET context = EXCLUDED.context, enabled = EXCLUDED.enabled, state = 'applying',
             state_reason = NULL, updated_by = EXCLUDED.updated_by, updated_at = now()",
        change.project.to_db(),
        change.pipeline.map(PipelineId::to_db),
        change.context,
        change.enabled,
        change.actor.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the pipelines whose rule must be on, in id order.
pub(super) async fn wanted(tx: &mut Tx<'_>, project: ProjectId) -> Result<Vec<Pipeline>, Error> {
    let rows = sqlx::query!(
        "SELECT p.id, p.name, p.file_key
         FROM merge_blocks b JOIN pipelines p ON p.id = b.pipeline_id
         WHERE b.project_id = $1 AND b.enabled
         ORDER BY p.id",
        project.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Pipeline {
            id: PipelineId::from_db(row.id),
            name: row.name,
            file_key: row.file_key,
        })
        .collect())
}

/// Writes a change of the GitHub ruleset: the rules of `applied` take their new report name and
/// the state `on`, the rules out of it that must be off take the state `off`, and the link
/// keeps the id of the ruleset. A rule that changed during the call keeps its state.
pub(super) async fn applied(
    tx: &mut Tx<'_>,
    project: ProjectId,
    applied: &[(PipelineId, String)],
    ruleset: Option<i64>,
) -> Result<(), Error> {
    // Keep the new names of the reports.
    let (ids, contexts): (Vec<Uuid>, Vec<String>) = applied
        .iter()
        .map(|(id, context)| (id.to_db(), context.clone()))
        .unzip();
    sqlx::query!(
        "UPDATE merge_blocks b SET context = w.context
         FROM unnest($2::uuid[], $3::text[]) AS w (id, context)
         WHERE b.project_id = $1 AND b.pipeline_id = w.id AND b.enabled
           AND b.context IS DISTINCT FROM w.context",
        project.to_db(),
        &ids,
        &contexts,
    )
    .execute(&mut **tx)
    .await?;

    // Write the states of the rules that the ruleset now has, or does not have.
    sqlx::query!(
        "UPDATE merge_blocks
         SET state = CASE WHEN enabled THEN 'on' ELSE 'off' END, state_reason = NULL,
             updated_at = now()
         WHERE project_id = $1 AND pipeline_id IS NOT NULL
           AND (pipeline_id = ANY($2::uuid[])) = enabled
           AND (state <> CASE WHEN enabled THEN 'on' ELSE 'off' END OR state_reason IS NOT NULL)",
        project.to_db(),
        &ids,
    )
    .execute(&mut **tx)
    .await?;

    // Keep the id of the ruleset.
    sqlx::query!(
        "UPDATE repository_links SET ruleset_id = $2, updated_at = now() WHERE project_id = $1",
        project.to_db(),
        ruleset,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Writes the state of one rule and its reason.
pub(super) async fn set_state(
    tx: &mut Tx<'_>,
    project: ProjectId,
    pipeline: Option<PipelineId>,
    state: MergeBlockState,
    reason: Option<&str>,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE merge_blocks SET state = $3, state_reason = $4, updated_at = now()
         WHERE project_id = $1 AND pipeline_id IS NOT DISTINCT FROM $2",
        project.to_db(),
        pipeline.map(PipelineId::to_db),
        state.as_str(),
        reason,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Makes a rule from the columns of its row.
fn block(project: ProjectId, columns: Columns) -> Result<MergeBlock, Error> {
    let state = MergeBlockState::parse(&columns.state)
        .ok_or_else(|| Error::Internal("unknown state of a merge rule".to_owned()))?;
    Ok(MergeBlock {
        project_id: project,
        pipeline_id: columns.pipeline.map(PipelineId::from_db),
        context: columns.context,
        enabled: columns.enabled,
        state,
        state_reason: columns.state_reason,
        updated_by: ActorId::from_db(columns.updated_by),
        updated_at: columns.updated_at,
    })
}
