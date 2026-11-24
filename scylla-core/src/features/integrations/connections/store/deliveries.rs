//! The SQL of the delivery counts of a workspace: the deliveries of each connection and the fires
//! of each webhook trigger, for each bucket of a period. A GitHub connection gets the deliveries
//! of its installation, and a GitLab connection the deliveries of its links. The buckets come as
//! times: the SQL knows no time zone.

use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::integrations::deliveries::{BucketCount, DeliveryFigures, LastDelivery};
use scylla_domain::projects::jobs::FiguresPeriod;
use scylla_domain::projects::triggers::TriggerId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::TimeZone;
use scylla_extension::Tx;
use time::OffsetDateTime;
use uuid::Uuid;

/// Returns the time zone of a workspace, or UTC for an unknown workspace.
pub(crate) async fn time_zone(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<TimeZone, sqlx::Error> {
    let name = sqlx::query_scalar!(
        "SELECT time_zone FROM workspaces WHERE id = $1",
        workspace.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(name
        .and_then(|name| TimeZone::parse(name).ok())
        .unwrap_or_default())
}

/// Returns the counts of each connection of a workspace in each bucket with a delivery, in the
/// order of the connections. `bounds` holds the start of the period, then the start of each next
/// bucket: number 1 counts the deliveries from the start of the period.
pub(crate) async fn connections(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    period: FiguresPeriod,
    bounds: &[OffsetDateTime],
) -> Result<Vec<BucketCount<VcsConnectionId>>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT c.id, width_bucket(d.received_at, $2::timestamptz[]) AS "bucket!",
                  count(*) AS "delivered!",
                  count(*) FILTER (WHERE d.error IS NOT NULL) AS "failed!",
                  max(d.received_at) AS "last!"
           FROM vcs_connections c
           JOIN vcs_deliveries d
             ON (c.provider = 'gitlab' AND d.connection_id = c.id)
             OR (c.provider = 'github' AND d.provider = 'github'
                 AND d.installation = c.external_id AND d.app_id IS NOT DISTINCT FROM c.app_id)
           WHERE c.workspace_id = $1 AND d.received_at >= $3 AND d.received_at < $4
           GROUP BY c.id, 2
           ORDER BY c.id, 2"#,
        workspace.to_db(),
        bounds,
        period.from,
        period.to,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let id = VcsConnectionId::from_db(row.id);
            count(id, row.bucket, row.delivered, row.failed, row.last)
        })
        .collect())
}

/// Returns the newest delivery of each connection of a workspace in a period.
pub(crate) async fn lasts(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    period: FiguresPeriod,
) -> Result<Vec<LastDelivery>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT DISTINCT ON (c.id) c.id, d.received_at, d.event, d.repository, d.ref
           FROM vcs_connections c
           JOIN vcs_deliveries d
             ON (c.provider = 'gitlab' AND d.connection_id = c.id)
             OR (c.provider = 'github' AND d.provider = 'github'
                 AND d.installation = c.external_id AND d.app_id IS NOT DISTINCT FROM c.app_id)
           WHERE c.workspace_id = $1 AND d.received_at >= $2 AND d.received_at < $3
           ORDER BY c.id, d.received_at DESC, d.delivery_id DESC"#,
        workspace.to_db(),
        period.from,
        period.to,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| LastDelivery {
            connection: VcsConnectionId::from_db(row.id),
            received_at: row.received_at,
            event: row.event,
            repository: row.repository,
            git_ref: row.r#ref,
        })
        .collect())
}

/// Returns the counts of the webhook fires of each trigger of a workspace in each bucket with a
/// fire, in the projects of `visible` (`None` for each project). `bounds` is as in
/// `connections`.
pub(crate) async fn triggers(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    visible: Option<&[ProjectId]>,
    period: FiguresPeriod,
    bounds: &[OffsetDateTime],
) -> Result<Vec<BucketCount<TriggerId>>, sqlx::Error> {
    let projects: Vec<Uuid> = visible
        .unwrap_or_default()
        .iter()
        .map(|id| id.to_db())
        .collect();
    let rows = sqlx::query!(
        r#"SELECT trigger_id, width_bucket(due_at, $2::timestamptz[]) AS "bucket!",
                  count(*) AS "delivered!",
                  count(*) FILTER (WHERE status = 'error') AS "failed!",
                  max(due_at) AS "last!"
           FROM trigger_fires
           WHERE workspace_id = $1 AND source = 'webhook'
             AND ($3::bool OR project_id = ANY($4::uuid[]))
             AND due_at >= $5 AND due_at < $6
           GROUP BY 1, 2"#,
        workspace.to_db(),
        bounds,
        visible.is_none(),
        &projects,
        period.from,
        period.to,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let id = TriggerId::from_db(row.trigger_id);
            count(id, row.bucket, row.delivered, row.failed, row.last)
        })
        .collect())
}

/// Makes the counts of one row, or `None` for a bucket number that is not a place.
fn count<K>(
    key: K,
    bucket: i32,
    delivered: i64,
    failed: i64,
    last: OffsetDateTime,
) -> Option<BucketCount<K>> {
    let number = |value: i64| u32::try_from(value).unwrap_or(u32::MAX);
    Some(BucketCount {
        key,
        bucket: usize::try_from(bucket).ok()?,
        figures: DeliveryFigures {
            delivered: number(delivered),
            failed: number(failed),
        },
        last_received_at: last,
    })
}
