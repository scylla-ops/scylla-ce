//! The SQL of the jobs that the agents hold: the jobs of a pipeline, of an agent, and of the
//! agents that went away.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_extension::Tx;
use time::OffsetDateTime;

/// Returns the jobs of a pipeline that an agent holds and that did not end, with their agent.
pub(crate) async fn held_in_pipeline(
    tx: &mut Tx<'_>,
    pipeline: PipelineId,
) -> Result<Vec<(JobId, AppId)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, agent_id AS "agent_id!" FROM jobs
           WHERE pipeline_id = $1 AND agent_id IS NOT NULL AND status IN ('pending', 'running')"#,
        pipeline.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (JobId::from_db(row.id), AppId::from_db(row.agent_id)))
        .collect())
}

/// Returns the ids of the jobs that an agent holds and that did not end.
pub(crate) async fn held_by(tx: &mut Tx<'_>, agent: AppId) -> Result<Vec<JobId>, sqlx::Error> {
    let ids = sqlx::query_scalar!(
        "SELECT id FROM jobs WHERE agent_id = $1 AND status IN ('pending', 'running') ORDER BY id",
        agent.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(ids.into_iter().map(JobId::from_db).collect())
}

/// Returns the jobs that did not end and whose agent was last seen before `before`, with their
/// agent.
pub(crate) async fn held_by_stale(
    tx: &mut Tx<'_>,
    before: OffsetDateTime,
) -> Result<Vec<(JobId, AppId)>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT j.id, a.id AS agent_id FROM jobs j JOIN apps a ON a.id = j.agent_id
         WHERE j.status IN ('pending', 'running')
           AND (a.last_seen_at IS NULL OR a.last_seen_at < $1)",
        before,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (JobId::from_db(row.id), AppId::from_db(row.agent_id)))
        .collect())
}
