//! The SQL of the cron schedules: the due triggers, their next fire, and the time zone of their
//! workspace.

use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::TriggerId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::TimeZone;
use scylla_extension::Tx;
use time::OffsetDateTime;

/// A cron trigger that is due.
pub(crate) struct Due {
    /// The trigger.
    pub(crate) id: TriggerId,
    /// The schedule.
    pub(crate) schedule: String,
    /// The time zone of the workspace.
    pub(crate) zone: TimeZone,
    /// The time of the fire that is due.
    pub(crate) due_at: OffsetDateTime,
    /// `true` when the pipeline is active: not archived, and with its file on the default branch.
    pub(crate) active: bool,
}

/// Returns a time zone of the database, or UTC for a name that the binary does not know.
fn zone_of(name: Option<String>) -> TimeZone {
    name.and_then(|name| TimeZone::parse(name).ok())
        .unwrap_or_default()
}

/// Locks at most `limit` enabled cron triggers that are due, oldest due time first. Another
/// server skips the locked rows.
pub(crate) async fn due(tx: &mut Tx<'_>, limit: i64) -> Result<Vec<Due>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT t.id, t.schedule AS "schedule!", t.next_fire_at AS "due_at!",
                  (SELECT w.time_zone FROM workspaces w WHERE w.id = t.workspace_id) AS zone,
                  (SELECT p.archived_at IS NULL AND p.removed_at IS NULL
                   FROM pipelines p WHERE p.id = t.pipeline_id) AS "active!"
           FROM triggers t
           WHERE t.enabled AND t.kind = 'cron' AND t.next_fire_at <= now()
           ORDER BY t.next_fire_at LIMIT $1 FOR UPDATE OF t SKIP LOCKED"#,
        limit,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Due {
            id: TriggerId::from_db(row.id),
            schedule: row.schedule,
            zone: zone_of(row.zone),
            due_at: row.due_at,
            active: row.active,
        })
        .collect())
}

/// Locks the enabled cron triggers of a workspace, and returns them with their schedules.
pub(crate) async fn crons(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<Vec<(TriggerId, String)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT id, schedule AS "schedule!" FROM triggers
           WHERE workspace_id = $1 AND enabled AND kind = 'cron'
           ORDER BY id FOR UPDATE"#,
        workspace.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (TriggerId::from_db(row.id), row.schedule))
        .collect())
}

/// Sets the next fire of a cron trigger.
pub(crate) async fn set_next(
    tx: &mut Tx<'_>,
    id: TriggerId,
    next: Option<OffsetDateTime>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE triggers SET next_fire_at = $2 WHERE id = $1",
        id.to_db(),
        next
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the time zone of a workspace, or UTC for an unknown workspace.
pub(crate) async fn zone(tx: &mut Tx<'_>, workspace: WorkspaceId) -> Result<TimeZone, sqlx::Error> {
    let name = sqlx::query_scalar!(
        "SELECT time_zone FROM workspaces WHERE id = $1",
        workspace.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(zone_of(name))
}

/// Returns the time zone of the workspace of a pipeline, or UTC for an unknown pipeline.
pub(crate) async fn pipeline_zone(
    tx: &mut Tx<'_>,
    pipeline: PipelineId,
) -> Result<TimeZone, sqlx::Error> {
    let name = sqlx::query_scalar!(
        "SELECT w.time_zone FROM pipelines p JOIN workspaces w ON w.id = p.workspace_id
         WHERE p.id = $1",
        pipeline.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(zone_of(name))
}
