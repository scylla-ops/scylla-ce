//! The SQL of the queue of the sender: the reports that cannot go out, the due reports that one
//! pass takes, what a call needs, and the result of a call.

use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::integrations::reports::ReportedStatus;
use scylla_domain::projects::jobs::JobId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// The error of a report that cannot go out because of its link or its connection.
const NOT_ACTIVE: &str = "the repository link or its connection is not active";

/// A due report that a pass took. Its time is the end of the call, so no other pass takes it.
pub(crate) struct Claimed {
    /// The job.
    pub(crate) job: JobId,
    /// The name of the report.
    pub(crate) context: String,
    /// The connection that sends it.
    pub(crate) connection: VcsConnectionId,
    /// The latest state of the run.
    pub(crate) status: ReportedStatus,
    /// The number of this state.
    pub(crate) sequence: i64,
    /// The failed calls for this state.
    pub(crate) attempts: i32,
    /// The id that the provider gave to the report: the GitLab pipeline of the status.
    pub(crate) external_id: Option<String>,
}

/// What a call sends, from the job, its commit, its link and its connection.
pub(crate) struct Facts {
    /// The start of the job on the agent.
    pub(crate) started_at: Option<OffsetDateTime>,
    /// The end of the job.
    pub(crate) finished_at: Option<OffsetDateTime>,
    /// The path of the pipeline file of the job.
    pub(crate) file_path: Option<String>,
    /// The first node that failed.
    pub(crate) failed_node: Option<String>,
    /// The commit.
    pub(crate) sha: String,
    /// The branch of the commit.
    pub(crate) branch: Option<String>,
    /// The tag of the commit.
    pub(crate) tag: Option<String>,
    /// The source branch of the first pull request.
    pub(crate) pull_branch: Option<String>,
    /// The full name of the repository.
    pub(crate) repository: String,
    /// The page of the repository.
    pub(crate) url: String,
    /// The pipeline of the job, as the text of its id.
    pub(crate) pipeline: String,
    /// The project of the job, as the text of its id.
    pub(crate) project: String,
    /// The slug of the workspace.
    pub(crate) slug: String,
    /// The provider.
    pub(crate) provider: String,
    /// The App of the instance of the connection.
    pub(crate) app_id: Option<uuid::Uuid>,
    /// The id of the connection on the provider: the installation on GitHub.
    pub(crate) account_id: String,
    /// The token of a GitLab connection, sealed with the id of the connection.
    pub(crate) sealed_token: Option<Vec<u8>>,
    /// The expiry of the token of the connection.
    pub(crate) token_expires_at: Option<OffsetDateTime>,
    /// The id of the repository on the provider.
    pub(crate) repository_id: String,
}

/// Stops each report that waits while its link or its connection is not active, or while its
/// link reports nothing. A call that runs keeps its report.
pub(crate) async fn stop_inactive(tx: &mut Tx<'_>) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE vcs_reports r
         SET state = 'not_sent', next_attempt_at = NULL, last_error = $1, updated_at = now()
         WHERE r.next_attempt_at IS NOT NULL
           AND (r.state <> 'sending' OR r.next_attempt_at <= now())
           AND NOT EXISTS (
               SELECT 1 FROM repository_links l JOIN vcs_connections c ON c.id = l.connection_id
               WHERE l.project_id = r.project_id AND c.id = r.connection_id
                 AND l.reports_enabled AND l.state = 'active' AND c.state = 'active')",
        NOT_ACTIVE,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Takes at most `limit` due reports, oldest first, and moves their time to the end of the call.
pub(crate) async fn claim(
    tx: &mut Tx<'_>,
    limit: i64,
    lease_seconds: f64,
) -> Result<Vec<Claimed>, Error> {
    let rows = sqlx::query!(
        "UPDATE vcs_reports r
         SET state = 'sending', next_attempt_at = now() + make_interval(secs => $2),
             updated_at = now()
         FROM (SELECT job_id, context FROM vcs_reports
               WHERE next_attempt_at <= now()
               ORDER BY next_attempt_at
               LIMIT $1
               FOR UPDATE SKIP LOCKED) due
         WHERE r.job_id = due.job_id AND r.context = due.context
         RETURNING r.job_id, r.context, r.connection_id, r.wanted_state, r.sequence, r.attempts,
                   r.external_id",
        limit,
        lease_seconds,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            let status = ReportedStatus::parse(&row.wanted_state)
                .ok_or_else(|| Error::Internal("unknown state of a report".to_owned()))?;
            Ok(Claimed {
                job: JobId::from_db(row.job_id),
                context: row.context,
                connection: VcsConnectionId::from_db(row.connection_id),
                status,
                sequence: row.sequence,
                attempts: row.attempts,
                external_id: row.external_id,
            })
        })
        .collect()
}

/// Returns what the call of a report needs, or `None` when its job, its commit or the link of
/// its connection is gone.
pub(crate) async fn facts(tx: &mut Tx<'_>, claimed: &Claimed) -> Result<Option<Facts>, Error> {
    let row = sqlx::query_as!(
        Facts,
        r#"SELECT j.started_at, j.finished_at,
                  j.pipeline_file->>'path' AS file_path,
                  (SELECT run->>'node' FROM jsonb_array_elements(j.runs) run
                   WHERE run->>'status' = 'failed' LIMIT 1) AS failed_node,
                  s.sha, s.branch, s.tag,
                  s.pull_requests->0->>'source_branch' AS pull_branch,
                  s.repository, s.url, j.pipeline_id::text AS "pipeline!",
                  j.project_id::text AS "project!", w.slug, c.provider, c.app_id,
                  c.external_id AS account_id, c.sealed_token, c.token_expires_at,
                  l.external_id AS repository_id
           FROM jobs j
           JOIN job_sources s ON s.job_id = j.id
           JOIN workspaces w ON w.id = j.workspace_id
           JOIN vcs_connections c ON c.id = $2
           JOIN repository_links l ON l.project_id = j.project_id AND l.connection_id = c.id
           WHERE j.id = $1"#,
        claimed.job.to_db(),
        claimed.connection.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row)
}

/// Keeps a sent state. The report is sent when its state did not change during the call, else
/// it is due again at once.
pub(crate) async fn sent(
    tx: &mut Tx<'_>,
    claimed: &Claimed,
    sent_state: &str,
    external_id: Option<&str>,
    external_url: Option<&str>,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE vcs_reports
         SET sent_state = $3, sent_sequence = $4, sent_at = now(),
             external_id = COALESCE($5, external_id), external_url = COALESCE($6, external_url),
             attempts = 0, last_error = NULL,
             state = CASE WHEN sequence = $4 THEN 'sent' ELSE 'sending' END,
             next_attempt_at = CASE WHEN sequence = $4 THEN NULL ELSE now() END,
             updated_at = now()
         WHERE job_id = $1 AND context = $2",
        claimed.job.to_db(),
        claimed.context,
        sent_state,
        claimed.sequence,
        external_id,
        external_url,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Keeps a failed call that the sender makes again at `at`, or stops the report after `max`
/// failed calls. Returns `true` when the report stopped.
pub(crate) async fn retry(
    tx: &mut Tx<'_>,
    claimed: &Claimed,
    at: OffsetDateTime,
    error: &str,
    max: i32,
) -> Result<bool, Error> {
    let state = sqlx::query_scalar!(
        "UPDATE vcs_reports
         SET attempts = attempts + 1, last_error = $3,
             state = CASE WHEN attempts + 1 >= $5 THEN 'not_sent' ELSE 'retrying' END,
             next_attempt_at = CASE WHEN attempts + 1 >= $5 THEN NULL ELSE $4::timestamptz END,
             updated_at = now()
         WHERE job_id = $1 AND context = $2
         RETURNING state",
        claimed.job.to_db(),
        claimed.context,
        error,
        at,
        max,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(state.as_deref() == Some("not_sent"))
}

/// Stops a report after a call that cannot work again. A report whose state changed during the
/// call is due again at once. Returns `true` when the report stopped.
pub(crate) async fn stop(tx: &mut Tx<'_>, claimed: &Claimed, error: &str) -> Result<bool, Error> {
    let state = sqlx::query_scalar!(
        "UPDATE vcs_reports
         SET attempts = attempts + 1, last_error = $4,
             state = CASE WHEN sequence = $3 THEN 'not_sent' ELSE 'sending' END,
             next_attempt_at = CASE WHEN sequence = $3 THEN NULL ELSE now() END,
             updated_at = now()
         WHERE job_id = $1 AND context = $2
         RETURNING state",
        claimed.job.to_db(),
        claimed.context,
        claimed.sequence,
        error,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(state.as_deref() == Some("not_sent"))
}
