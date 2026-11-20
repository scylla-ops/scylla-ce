//! The open items of the `trigger_fires` table: the fires that failed and the missed times after
//! the last good fire of their trigger, and the times when such items went away.

use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{FireId, FireSource, TriggerId};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::dashboards::attention::{
    AttentionItem, AttentionKind, AttentionQuery, ItemFire, Place, Subject,
};
use scylla_extension::Tx;
use time::OffsetDateTime;
use uuid::Uuid;

use super::jobs::places;
use crate::Error;

/// A row of a fire item.
struct Row {
    /// The id of the fire.
    id: Uuid,
    /// The trigger.
    trigger_id: Uuid,
    /// The display name of the trigger.
    trigger_name: String,
    /// The cron schedule of the trigger.
    schedule: Option<String>,
    /// The pipeline.
    pipeline_id: Uuid,
    /// The display name of the pipeline.
    pipeline_name: String,
    /// The project.
    project_id: Uuid,
    /// The display name of the project.
    project_name: String,
    /// What started the fire.
    source: String,
    /// The due time.
    due_at: OffsetDateTime,
    /// The result: `error` or `missed`.
    status: String,
    /// The reason of a fire that failed.
    error: Option<String>,
    /// The open items of the same trigger and kind.
    count: i64,
}

impl Row {
    /// Makes the item of the row.
    fn into_item(self) -> Result<AttentionItem, Error> {
        let bad = |what: &str| Error::Internal(format!("bad trigger fire data: {what}"));
        let kind = match self.status.as_str() {
            "error" => AttentionKind::FireFailed,
            "missed" => AttentionKind::FireMissed,
            _ => return Err(bad("status")),
        };
        Ok(AttentionItem {
            kind,
            at: self.due_at,
            count: u32::try_from(self.count).unwrap_or(u32::MAX),
            is_new: false,
            place: Some(Place {
                project_id: ProjectId::from_db(self.project_id),
                project_name: self.project_name,
                pipeline_id: PipelineId::from_db(self.pipeline_id),
                pipeline_name: self.pipeline_name,
            }),
            subject: Subject::Fire(ItemFire {
                id: FireId::from_db(self.id),
                trigger_id: TriggerId::from_db(self.trigger_id),
                trigger_name: self.trigger_name,
                schedule: self.schedule,
                source: FireSource::parse(&self.source).ok_or_else(|| bad("source"))?,
                error: self.error,
            }),
        })
    }
}

/// Returns the newest fire of each trigger and result of `statuses` due at `from` or later, with
/// no later good fire of the trigger. A trigger of an archived pipeline has none.
pub(crate) async fn open(
    tx: &mut Tx<'_>,
    query: &AttentionQuery,
    statuses: &[&str],
    from: OffsetDateTime,
) -> Result<Vec<AttentionItem>, Error> {
    let (all, visible, projects) = places(&query.reach.fires, query);
    let rows = sqlx::query_as!(
        Row,
        r#"WITH bad AS (
               SELECT f.id, f.trigger_id, f.pipeline_id, f.project_id, f.source, f.due_at,
                      f.status, f.error, row_number() OVER w AS rank, count(*) OVER w AS count
               FROM trigger_fires f
               JOIN pipelines p ON p.id = f.pipeline_id
               WHERE f.workspace_id = $1 AND f.status = ANY($2::text[]) AND f.due_at >= $3
                 AND p.archived_at IS NULL AND p.removed_at IS NULL
                 AND ($4::bool OR f.project_id = ANY($5::uuid[]))
                 AND (cardinality($6::uuid[]) = 0 OR f.project_id = ANY($6))
                 AND NOT EXISTS (
                     SELECT 1 FROM trigger_fires g
                     WHERE g.trigger_id = f.trigger_id AND g.due_at > f.due_at
                       AND g.status = 'ok')
               WINDOW w AS (PARTITION BY f.trigger_id, f.status ORDER BY f.due_at DESC, f.id DESC
                             ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING)
           )
           SELECT b.id AS "id!", b.trigger_id AS "trigger_id!", t.name AS trigger_name,
                  t.schedule, b.pipeline_id AS "pipeline_id!", p.name AS pipeline_name,
                  b.project_id AS "project_id!", r.name AS project_name, b.source AS "source!",
                  b.due_at AS "due_at!", b.status AS "status!", b.error AS "error?",
                  b.count AS "count!"
           FROM bad b
           JOIN triggers t ON t.id = b.trigger_id
           JOIN pipelines p ON p.id = b.pipeline_id
           JOIN projects r ON r.id = b.project_id
           WHERE b.rank = 1"#,
        query.workspace.to_db(),
        statuses as &[&str],
        from,
        all,
        &visible,
        &projects,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_item).collect()
}

/// Returns the last start of a good fire that came after a fire of `statuses` of the same
/// trigger, from `from`.
pub(crate) async fn cleared(
    tx: &mut Tx<'_>,
    query: &AttentionQuery,
    statuses: &[&str],
    from: OffsetDateTime,
) -> Result<Option<OffsetDateTime>, sqlx::Error> {
    let (all, visible, projects) = places(&query.reach.fires, query);
    sqlx::query_scalar!(
        r#"SELECT max(t.fired_at) AS at FROM (
               SELECT f.status, f.fired_at,
                      lag(f.status) OVER (PARTITION BY f.trigger_id ORDER BY f.due_at, f.id)
                          AS before
               FROM trigger_fires f
               JOIN pipelines p ON p.id = f.pipeline_id
               WHERE f.workspace_id = $1 AND f.due_at >= $3
                 AND p.archived_at IS NULL AND p.removed_at IS NULL
                 AND ($4::bool OR f.project_id = ANY($5::uuid[]))
                 AND (cardinality($6::uuid[]) = 0 OR f.project_id = ANY($6))
           ) t
           WHERE t.status = 'ok' AND t.before = ANY($2::text[])"#,
        query.workspace.to_db(),
        statuses as &[&str],
        from,
        all,
        &visible,
        &projects,
    )
    .fetch_one(&mut **tx)
    .await
}
