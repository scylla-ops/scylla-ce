//! The SQL of the GitHub check runs: the last log lines of a failed node, and the jobs that a
//! person asks GitHub to run again.

use scylla_domain::projects::jobs::JobId;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;

use crate::Error;

/// Returns the last `limit` log lines of a node of a job, oldest first. Each batch has one line
/// at least, so the last `limit` batches hold the lines.
pub(crate) async fn tail(
    tx: &mut Tx<'_>,
    job: JobId,
    node: &str,
    limit: i64,
) -> Result<Vec<String>, Error> {
    let mut lines = sqlx::query_scalar!(
        r#"SELECT item.line->>'text' AS "text!"
           FROM (SELECT id, lines FROM job_logs
                 WHERE job_id = $1 AND node_id = $2
                 ORDER BY id DESC LIMIT $3) batch,
                jsonb_array_elements(batch.lines) WITH ORDINALITY AS item(line, place)
           ORDER BY batch.id DESC, item.place DESC
           LIMIT $3"#,
        job.to_db(),
        node,
        limit,
    )
    .fetch_all(&mut **tx)
    .await?;
    lines.reverse();
    Ok(lines)
}

/// Returns the jobs of a project to run again for a commit: the newest job of each check run of
/// the commit, or only the job `check` when it is set. A job that did not end is left out.
pub(crate) async fn reruns(
    tx: &mut Tx<'_>,
    project: ProjectId,
    sha: &str,
    check: Option<JobId>,
) -> Result<Vec<JobId>, Error> {
    let rows = sqlx::query_scalar!(
        r#"SELECT newest.job_id AS "job_id!"
           FROM (SELECT DISTINCT ON (r.context) r.job_id, j.status
                 FROM vcs_reports r
                 JOIN jobs j ON j.id = r.job_id
                 JOIN job_sources s ON s.job_id = r.job_id
                 WHERE r.project_id = $1 AND r.mode = 'check_run' AND s.sha = $2
                   AND ($3::uuid IS NULL OR r.job_id = $3)
                 ORDER BY r.context, j.created_at DESC, j.id DESC) newest
           WHERE newest.status NOT IN ('awaiting_approval', 'pending', 'running')
           ORDER BY newest.job_id"#,
        project.to_db(),
        sha,
        check.map(JobId::to_db),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(JobId::from_db).collect())
}
