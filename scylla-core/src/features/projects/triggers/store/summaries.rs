//! The SQL of the summaries of the triggers of a workspace: each trigger with the names of its
//! pipeline and of its project, and the state of the newest job of each trigger.

use std::collections::HashMap;

use scylla_domain::projects::jobs::JobStatus;
use scylla_domain::projects::triggers::{ListWorkspaceTriggers, TriggerId, TriggerSummary};
use scylla_extension::Tx;
use uuid::Uuid;

use super::row::Row;
use crate::Error;

/// Returns the summary of each trigger of a workspace that the query keeps, with no next fire
/// and no result. The triggers of the `on` table of a file pipeline are not in it.
pub(crate) async fn all(
    tx: &mut Tx<'_>,
    query: &ListWorkspaceTriggers,
) -> Result<Vec<TriggerSummary>, Error> {
    // Read one row for each trigger.
    let visible: Vec<Uuid> = query
        .visible
        .iter()
        .flatten()
        .map(|id| id.to_db())
        .collect();
    let projects: Vec<Uuid> = query.projects.iter().map(|id| id.to_db()).collect();
    let pipelines: Vec<Uuid> = query.pipelines.iter().map(|id| id.to_db()).collect();
    let kinds: Vec<&str> = query.kinds.iter().map(|kind| kind.as_str()).collect();
    let rows = sqlx::query!(
        r#"SELECT t.id, t.pipeline_id, t.project_id, t.workspace_id, t.created_by, t.name,
                  t.kind, t.schedule, t.rules, t.inputs, t.enabled, t.next_fire_at,
                  t.last_fired_at, t.last_status, t.last_error, t.created_at, t.updated_at,
                  p.name AS pipeline_name, r.name AS project_name,
                  p.archived_at IS NULL AND p.removed_at IS NULL AS "active!"
           FROM triggers t
           JOIN pipelines p ON p.id = t.pipeline_id
           JOIN projects r ON r.id = t.project_id
           WHERE t.workspace_id = $1
             AND ($2::bool OR t.project_id = ANY($3::uuid[]))
             AND (cardinality($4::uuid[]) = 0 OR t.project_id = ANY($4))
             AND (cardinality($5::uuid[]) = 0 OR t.pipeline_id = ANY($5))
             AND (cardinality($6::text[]) = 0 OR t.kind = ANY($6))
             AND (NOT $7 OR t.enabled)"#,
        query.workspace.to_db(),
        query.visible.is_none(),
        &visible,
        &projects,
        &pipelines,
        &kinds as &[&str],
        query.enabled_only,
    )
    .fetch_all(&mut **tx)
    .await?;

    // Make the summaries.
    rows.into_iter()
        .map(|row| {
            let trigger = Row {
                id: row.id,
                pipeline_id: row.pipeline_id,
                project_id: row.project_id,
                workspace_id: row.workspace_id,
                created_by: row.created_by,
                name: row.name,
                kind: row.kind,
                schedule: row.schedule,
                rules: row.rules,
                inputs: row.inputs,
                enabled: row.enabled,
                next_fire_at: row.next_fire_at,
                last_fired_at: row.last_fired_at,
                last_status: row.last_status,
                last_error: row.last_error,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }
            .into_trigger()?;
            Ok(TriggerSummary {
                trigger,
                pipeline_name: row.pipeline_name,
                project_name: row.project_name,
                active: row.active,
                upcoming: Vec::new(),
                last_job_status: None,
                last_fire: None,
                failed_fires: 0,
                missed_times: 0,
            })
        })
        .collect()
}

/// Returns the state of the newest job of each trigger that has a job.
pub(crate) async fn last_jobs(
    tx: &mut Tx<'_>,
    triggers: &[TriggerId],
) -> Result<HashMap<TriggerId, JobStatus>, Error> {
    let ids: Vec<Uuid> = triggers.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"SELECT DISTINCT ON (trigger_id) trigger_id AS "trigger_id!", status
           FROM jobs WHERE trigger_id = ANY($1)
           ORDER BY trigger_id, id DESC"#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            let status = JobStatus::parse(&row.status)
                .ok_or_else(|| Error::Internal(format!("unknown job state {}", row.status)))?;
            Ok((TriggerId::from_db(row.trigger_id), status))
        })
        .collect()
}
