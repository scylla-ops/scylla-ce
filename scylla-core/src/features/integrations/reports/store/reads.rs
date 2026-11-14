//! The SQL of the routes of the reports: the place of the jobs, their reports, why a job has
//! none, and a report that a person sends again.

use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::integrations::reports::{ReportMode, ReportState, ReportedStatus, VcsReport};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// A report with the provider of its connection.
pub(crate) struct Listed {
    /// The report.
    pub(crate) report: VcsReport,
    /// The provider of its connection.
    pub(crate) provider: VcsProvider,
}

/// What tells why a job has no report.
pub(crate) struct Why {
    /// The job.
    pub(crate) job: JobId,
    /// `true` when the job has a commit.
    pub(crate) source: bool,
    /// `true` when the project has a link.
    pub(crate) linked: bool,
    /// `true` when the link reports.
    pub(crate) reports: bool,
    /// `true` when the link and its connection are active.
    pub(crate) active: bool,
}

/// One row of `vcs_reports`, with the provider of its connection.
struct Row {
    /// The job.
    job_id: Uuid,
    /// The name of the report.
    context: String,
    /// The project.
    project_id: Uuid,
    /// The connection.
    connection_id: Uuid,
    /// The provider of the connection.
    provider: String,
    /// The mode.
    mode: String,
    /// The latest state of the run.
    wanted_state: String,
    /// The number of the latest state.
    sequence: i64,
    /// The delivery state.
    state: String,
    /// The last state that the provider got.
    sent_state: Option<String>,
    /// The time of the last send.
    sent_at: Option<OffsetDateTime>,
    /// The id on the provider.
    external_id: Option<String>,
    /// The page on the provider.
    external_url: Option<String>,
    /// The failed calls of the latest state.
    attempts: i32,
    /// The time of the next call.
    next_attempt_at: Option<OffsetDateTime>,
    /// The last error.
    last_error: Option<String>,
}

/// Returns the project and the workspace of each known job of a list.
pub(crate) async fn places(
    tx: &mut Tx<'_>,
    jobs: &[JobId],
) -> Result<Vec<(JobId, ProjectId, WorkspaceId)>, Error> {
    let ids: Vec<Uuid> = jobs.iter().map(|job| job.to_db()).collect();
    let rows = sqlx::query!(
        "SELECT id, project_id, workspace_id FROM jobs WHERE id = ANY($1)",
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                JobId::from_db(row.id),
                ProjectId::from_db(row.project_id),
                WorkspaceId::from_db(row.workspace_id),
            )
        })
        .collect())
}

/// Returns the reports of jobs, by job and name.
pub(crate) async fn list(tx: &mut Tx<'_>, jobs: &[JobId]) -> Result<Vec<Listed>, Error> {
    let ids: Vec<Uuid> = jobs.iter().map(|job| job.to_db()).collect();
    let rows = sqlx::query_as!(
        Row,
        "SELECT r.job_id, r.context, r.project_id, r.connection_id, c.provider, r.mode,
                r.wanted_state, r.sequence, r.state, r.sent_state, r.sent_at, r.external_id,
                r.external_url, r.attempts, r.next_attempt_at, r.last_error
         FROM vcs_reports r JOIN vcs_connections c ON c.id = r.connection_id
         WHERE r.job_id = ANY($1)
         ORDER BY r.job_id, r.context",
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(listed).collect()
}

/// Returns what tells why each job of a list has no report.
pub(crate) async fn why(tx: &mut Tx<'_>, jobs: &[JobId]) -> Result<Vec<Why>, Error> {
    let ids: Vec<Uuid> = jobs.iter().map(|job| job.to_db()).collect();
    let rows = sqlx::query!(
        r#"SELECT j.id AS "id!", s.job_id IS NOT NULL AS "source!", l.project_id IS NOT NULL AS "linked!",
                  COALESCE(l.reports_enabled, false) AS "reports!",
                  COALESCE(l.state = 'active' AND c.state = 'active', false) AS "active!"
           FROM jobs j
           LEFT JOIN job_sources s ON s.job_id = j.id
           LEFT JOIN repository_links l ON l.project_id = j.project_id
           LEFT JOIN vcs_connections c ON c.id = l.connection_id
           WHERE j.id = ANY($1)"#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Why {
            job: JobId::from_db(row.id),
            source: row.source,
            linked: row.linked,
            reports: row.reports,
            active: row.active,
        })
        .collect())
}

/// Makes a report that waits or that stopped due at once, with no failed call. A report that is
/// sent or that is sending does not change. Returns the report, or `None` for an unknown name.
pub(crate) async fn retry(
    tx: &mut Tx<'_>,
    job: JobId,
    context: &str,
) -> Result<Option<Listed>, Error> {
    sqlx::query!(
        "UPDATE vcs_reports
         SET state = 'sending', attempts = 0, last_error = NULL, next_attempt_at = now(),
             updated_at = now()
         WHERE job_id = $1 AND context = $2 AND state IN ('retrying', 'not_sent')",
        job.to_db(),
        context,
    )
    .execute(&mut **tx)
    .await?;
    let row = sqlx::query_as!(
        Row,
        "SELECT r.job_id, r.context, r.project_id, r.connection_id, c.provider, r.mode,
                r.wanted_state, r.sequence, r.state, r.sent_state, r.sent_at, r.external_id,
                r.external_url, r.attempts, r.next_attempt_at, r.last_error
         FROM vcs_reports r JOIN vcs_connections c ON c.id = r.connection_id
         WHERE r.job_id = $1 AND r.context = $2",
        job.to_db(),
        context,
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(listed).transpose()
}

/// Reads a row. An unknown value in the database gives `INTERNAL`.
fn listed(row: Row) -> Result<Listed, Error> {
    let unknown = |what: &str| Error::Internal(format!("unknown {what} of a report"));
    let report = VcsReport {
        job_id: JobId::from_db(row.job_id),
        context: row.context,
        project_id: ProjectId::from_db(row.project_id),
        connection_id: VcsConnectionId::from_db(row.connection_id),
        mode: ReportMode::parse(&row.mode).ok_or_else(|| unknown("mode"))?,
        wanted: ReportedStatus::parse(&row.wanted_state).ok_or_else(|| unknown("run state"))?,
        sequence: row.sequence,
        state: ReportState::parse(&row.state).ok_or_else(|| unknown("state"))?,
        sent_state: row.sent_state,
        sent_at: row.sent_at,
        external_id: row.external_id,
        external_url: row.external_url,
        attempts: u32::try_from(row.attempts).unwrap_or_default(),
        next_attempt_at: row.next_attempt_at,
        last_error: row.last_error,
    };
    Ok(Listed {
        report,
        provider: VcsProvider::parse(&row.provider).ok_or_else(|| unknown("provider"))?,
    })
}
