//! The SQL of the outbox: the link and the pipeline of a job, and the upsert of the state that its
//! report must send.

use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::integrations::links::ReportMode;
use scylla_domain::integrations::reports::ReportedStatus;
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;

use super::queue::NOT_ACTIVE;
use crate::Error;

/// The link of the project of a job, and the pipeline of the job.
pub(crate) struct Target {
    /// The provider of the connection.
    pub(crate) provider: VcsProvider,
    /// The connection of the link.
    pub(crate) connection: VcsConnectionId,
    /// How the link reports.
    pub(crate) mode: ReportMode,
    /// `true` when the link reports.
    pub(crate) reports: bool,
    /// `true` when the link and its connection are active.
    pub(crate) active: bool,
    /// The name of the pipeline.
    pub(crate) name: String,
    /// The key of a file pipeline, or `None` for a pipeline of the editor.
    pub(crate) file_key: Option<String>,
}

/// The state that the report of a pipeline of a job must send.
pub(crate) struct Wanted {
    /// The job.
    pub(crate) job: JobId,
    /// The name of the report.
    pub(crate) context: String,
    /// The project of the job.
    pub(crate) project: ProjectId,
    /// The connection that sends the report.
    pub(crate) connection: VcsConnectionId,
    /// How the report goes to the provider.
    pub(crate) mode: ReportMode,
    /// The state of the run.
    pub(crate) status: ReportedStatus,
    /// `true` when the link and its connection are active.
    pub(crate) active: bool,
}

/// Returns the link of a project with the pipeline of a job, or `None` for a project with no
/// link.
pub(crate) async fn target(
    tx: &mut Tx<'_>,
    project: ProjectId,
    pipeline: PipelineId,
) -> Result<Option<Target>, Error> {
    let row = sqlx::query!(
        r#"SELECT c.id AS connection_id, c.provider, l.report_mode, p.name, p.file_key,
                  l.reports_enabled AS reports,
                  (l.state = 'active' AND c.state = 'active') AS "active!"
           FROM repository_links l
           JOIN vcs_connections c ON c.id = l.connection_id
           JOIN pipelines p ON p.id = $2 AND p.project_id = l.project_id
           WHERE l.project_id = $1"#,
        project.to_db(),
        pipeline.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let unknown = |what: &str| Error::Internal(format!("unknown {what} of a repository link"));
    Ok(Some(Target {
        provider: VcsProvider::parse(&row.provider).ok_or_else(|| unknown("provider"))?,
        connection: VcsConnectionId::from_db(row.connection_id),
        mode: ReportMode::parse(&row.report_mode).ok_or_else(|| unknown("report mode"))?,
        reports: row.reports,
        active: row.active,
        name: row.name,
        file_key: row.file_key,
    }))
}

/// Records the state that a report must send: a new report, or a new state with one more in its
/// sequence. A state that the report does not take changes nothing. A report that waits keeps
/// its time, so a call that runs or a limit of the provider keeps its place. While the link or
/// its connection is not active, no report starts, and a report keeps its new state as not sent.
pub(crate) async fn record(tx: &mut Tx<'_>, wanted: &Wanted) -> Result<(), Error> {
    // Read the state of the report, with its row locked.
    let job = wanted.job.to_db();
    let old = sqlx::query_scalar!(
        "SELECT wanted_state FROM vcs_reports WHERE job_id = $1 AND context = $2 FOR UPDATE",
        job,
        wanted.context,
    )
    .fetch_optional(&mut **tx)
    .await?;

    // Insert a new report, only for an active link and connection.
    let Some(old) = old else {
        if !wanted.active {
            return Ok(());
        }
        sqlx::query!(
            "INSERT INTO vcs_reports (job_id, context, project_id, connection_id, mode,
                                      wanted_state, sequence, next_attempt_at)
             VALUES ($1, $2, $3, $4, $5, $6, 1, now())
             ON CONFLICT (job_id, context) DO NOTHING",
            job,
            wanted.context,
            wanted.project.to_db(),
            wanted.connection.to_db(),
            wanted.mode.as_str(),
            wanted.status.as_str(),
        )
        .execute(&mut **tx)
        .await?;
        return Ok(());
    };

    // Else take the new state, if the report takes it.
    let old = ReportedStatus::parse(&old)
        .ok_or_else(|| Error::Internal("unknown state of a report".to_owned()))?;
    if !old.takes(wanted.status) {
        return Ok(());
    }
    sqlx::query!(
        "UPDATE vcs_reports
         SET wanted_state = $3, sequence = sequence + 1, attempts = 0,
             last_error = CASE WHEN $4::bool THEN NULL ELSE $5::text END,
             state = CASE WHEN NOT $4::bool THEN 'not_sent'
                          WHEN state IN ('sent', 'not_sent') THEN 'sending' ELSE state END,
             next_attempt_at = CASE WHEN NOT $4::bool THEN NULL
                                    WHEN state IN ('sent', 'not_sent') THEN now()
                                    ELSE COALESCE(next_attempt_at, now()) END,
             updated_at = now()
         WHERE job_id = $1 AND context = $2",
        job,
        wanted.context,
        wanted.status.as_str(),
        wanted.active,
        NOT_ACTIVE,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
