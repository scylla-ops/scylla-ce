//! The SQL of the `trigger_fires` table: the history of the fires of the triggers and of the cron
//! times that the server missed.

use std::collections::HashMap;

use scylla_domain::projects::jobs::{JobId, JobStatus};
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{
    FireId, FireResult, FireSource, FireStatus, ListFires, TriggerFire, TriggerId,
};
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// A fire to record.
pub(crate) struct NewFire<'a> {
    /// The trigger.
    pub(crate) trigger: TriggerId,
    /// What started the fire.
    pub(crate) source: FireSource,
    /// The time when the fire was due.
    pub(crate) due_at: OffsetDateTime,
    /// The start of the fire.
    pub(crate) fired_at: OffsetDateTime,
    /// The result.
    pub(crate) result: &'a FireResult,
    /// The job of the fire.
    pub(crate) job: Option<JobId>,
}

/// A row of a fire, with the name of its trigger and the state of its job.
struct Row {
    /// The id.
    id: Uuid,
    /// The trigger.
    trigger_id: Uuid,
    /// The display name of the trigger.
    trigger_name: String,
    /// The pipeline.
    pipeline_id: Uuid,
    /// The project.
    project_id: Uuid,
    /// The source.
    source: String,
    /// The due time.
    due_at: OffsetDateTime,
    /// The start of the fire.
    fired_at: Option<OffsetDateTime>,
    /// The result.
    status: String,
    /// The reason of a failed fire.
    error: Option<String>,
    /// The job.
    job_id: Option<Uuid>,
    /// The state of the job.
    job_status: Option<String>,
}

impl Row {
    /// Makes the fire of the row.
    fn into_fire(self) -> Result<TriggerFire, Error> {
        let bad = |what: &str| Error::Internal(format!("bad trigger fire data: {what}"));
        let job_status = match self.job_status.as_deref() {
            Some(name) => Some(JobStatus::parse(name).ok_or_else(|| bad("job status"))?),
            None => None,
        };
        Ok(TriggerFire {
            id: FireId::from_db(self.id),
            trigger_id: TriggerId::from_db(self.trigger_id),
            trigger_name: self.trigger_name,
            pipeline_id: PipelineId::from_db(self.pipeline_id),
            project_id: ProjectId::from_db(self.project_id),
            source: FireSource::parse(&self.source).ok_or_else(|| bad("source"))?,
            due_at: self.due_at,
            fired_at: self.fired_at,
            status: FireStatus::parse(&self.status).ok_or_else(|| bad("status"))?,
            error: self.error,
            job_id: self.job_id.map(JobId::from_db),
            job_status,
        })
    }
}

/// Records a fire of a trigger with the place of the trigger. A deleted trigger records nothing.
pub(crate) async fn insert(tx: &mut Tx<'_>, fire: NewFire<'_>) -> Result<(), sqlx::Error> {
    let (status, error) = match fire.result {
        FireResult::Ok => (FireStatus::Ok, None),
        FireResult::Error(reason) => (FireStatus::Error, Some(reason.as_str())),
    };
    sqlx::query!(
        "INSERT INTO trigger_fires (id, trigger_id, pipeline_id, project_id, workspace_id, source,
                                    due_at, fired_at, status, error, job_id)
         SELECT $1, t.id, t.pipeline_id, t.project_id, t.workspace_id, $3, $4, $5, $6, $7, $8
         FROM triggers t WHERE t.id = $2",
        FireId::generate().to_db(),
        fire.trigger.to_db(),
        fire.source.as_str(),
        fire.due_at,
        fire.fired_at,
        status.as_str(),
        error,
        fire.job.map(JobId::to_db),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Records the cron times of a trigger that the server missed.
pub(crate) async fn insert_missed(
    tx: &mut Tx<'_>,
    trigger: TriggerId,
    times: &[OffsetDateTime],
) -> Result<(), sqlx::Error> {
    if times.is_empty() {
        return Ok(());
    }
    let ids: Vec<Uuid> = times.iter().map(|_| FireId::generate().to_db()).collect();
    sqlx::query!(
        "INSERT INTO trigger_fires (id, trigger_id, pipeline_id, project_id, workspace_id, source,
                                    due_at, status)
         SELECT m.id, t.id, t.pipeline_id, t.project_id, t.workspace_id, 'schedule', m.due_at,
                'missed'
         FROM triggers t, unnest($2::uuid[], $3::timestamptz[]) AS m (id, due_at)
         WHERE t.id = $1",
        trigger.to_db(),
        &ids,
        times,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the fires due before `before`, and returns their count.
pub(crate) async fn purge(tx: &mut Tx<'_>, before: OffsetDateTime) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM trigger_fires WHERE due_at < $1", before)
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected())
}

/// Returns the fires of a list after the fire of the page, newest due time first, with one extra
/// row.
pub(crate) async fn list(tx: &mut Tx<'_>, query: &ListFires) -> Result<Vec<TriggerFire>, Error> {
    let fires = &query.fires;
    let visible: Vec<Uuid> = fires
        .visible
        .iter()
        .flatten()
        .map(|id| id.to_db())
        .collect();
    let projects: Vec<Uuid> = fires.projects.iter().map(|id| id.to_db()).collect();
    let pipelines: Vec<Uuid> = fires.pipelines.iter().map(|id| id.to_db()).collect();
    let triggers: Vec<Uuid> = fires.triggers.iter().map(|id| id.to_db()).collect();
    let sources: Vec<&str> = fires.sources.iter().map(|one| one.as_str()).collect();
    let kinds: Vec<&str> = query.kinds.iter().map(|one| one.as_str()).collect();
    let statuses: Vec<&str> = query.statuses.iter().map(|one| one.as_str()).collect();
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT f.id, f.trigger_id, t.name AS trigger_name, f.pipeline_id, f.project_id,
                  f.source, f.due_at, f.fired_at, f.status, f.error, f.job_id,
                  j.status AS "job_status?"
           FROM trigger_fires f
           JOIN triggers t ON t.id = f.trigger_id
           LEFT JOIN jobs j ON j.id = f.job_id
           WHERE f.workspace_id = $1
             AND ($2::bool OR f.project_id = ANY($3::uuid[]))
             AND (cardinality($4::uuid[]) = 0 OR f.project_id = ANY($4))
             AND (cardinality($5::uuid[]) = 0 OR f.pipeline_id = ANY($5))
             AND (cardinality($6::uuid[]) = 0 OR f.trigger_id = ANY($6))
             AND (cardinality($7::text[]) = 0 OR f.source = ANY($7))
             AND (cardinality($8::text[]) = 0 OR t.kind = ANY($8))
             AND (cardinality($9::text[]) = 0 OR f.status = ANY($9))
             AND ($10::timestamptz IS NULL OR f.due_at >= $10)
             AND ($11::timestamptz IS NULL OR f.due_at < $11)
             AND ($12::uuid IS NULL OR (f.due_at, f.id) <
                  (SELECT a.due_at, a.id FROM trigger_fires a WHERE a.id = $12))
           ORDER BY f.due_at DESC, f.id DESC LIMIT $13"#,
        fires.workspace.to_db(),
        fires.visible.is_none(),
        &visible,
        &projects,
        &pipelines,
        &triggers,
        &sources as &[&str],
        &kinds as &[&str],
        &statuses as &[&str],
        query.from,
        query.to,
        query.page.after.map(FireId::to_db),
        query.page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_fire).collect()
}

/// Returns the newest fire of each trigger that has one.
pub(crate) async fn last_of(
    tx: &mut Tx<'_>,
    triggers: &[TriggerId],
) -> Result<HashMap<TriggerId, TriggerFire>, Error> {
    let ids: Vec<Uuid> = triggers.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT DISTINCT ON (f.trigger_id)
                  f.id, f.trigger_id, t.name AS trigger_name, f.pipeline_id, f.project_id,
                  f.source, f.due_at, f.fired_at, f.status, f.error, f.job_id,
                  j.status AS "job_status?"
           FROM trigger_fires f
           JOIN triggers t ON t.id = f.trigger_id
           LEFT JOIN jobs j ON j.id = f.job_id
           WHERE f.trigger_id = ANY($1)
           ORDER BY f.trigger_id, f.due_at DESC, f.id DESC"#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| row.into_fire().map(|fire| (fire.trigger_id, fire)))
        .collect()
}

/// Returns the failed fires and the missed times of each trigger with one since a time.
pub(crate) async fn problems(
    tx: &mut Tx<'_>,
    triggers: &[TriggerId],
    since: OffsetDateTime,
) -> Result<HashMap<TriggerId, (u32, u32)>, sqlx::Error> {
    let ids: Vec<Uuid> = triggers.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"SELECT trigger_id,
                  count(*) FILTER (WHERE status = 'error') AS "failed!",
                  count(*) FILTER (WHERE status = 'missed') AS "missed!"
           FROM trigger_fires
           WHERE trigger_id = ANY($1) AND due_at >= $2 AND status IN ('error', 'missed')
           GROUP BY trigger_id"#,
        &ids,
        since,
    )
    .fetch_all(&mut **tx)
    .await?;
    let count = |value: i64| u32::try_from(value).unwrap_or(u32::MAX);
    Ok(rows
        .into_iter()
        .map(|row| {
            let counts = (count(row.failed), count(row.missed));
            (TriggerId::from_db(row.trigger_id), counts)
        })
        .collect())
}
