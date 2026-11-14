//! The jobs of the commits of the Git events: an automatic event joins the job of its pipeline and
//! commit, and a new commit of a pull request from a fork cancels the jobs of its older commits
//! that wait for an approval.

use scylla_domain::integrations::sources::JobSource;
use scylla_domain::projects::jobs::{JobId, JobStatus};
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use sqlx::PgPool;

use super::runs::now;
use super::store::{jobs, sources};
use crate::features::integrations::reports;
use crate::Error;

/// Adds the branch and the pull requests of an automatic source to the job of the same pipeline
/// and commit. Returns that job, or `None` when the pipeline has no automatic job of the commit.
pub(crate) async fn join(
    pool: &PgPool,
    pipeline: PipelineId,
    source: &JobSource,
) -> Result<Option<JobId>, Error> {
    let mut tx = pool.begin().await?;
    let job = sources::join(&mut tx, pipeline, source).await?;
    tx.commit().await?;
    Ok(job)
}

/// Returns `true` for the failure of a new automatic job whose pipeline and commit have a job:
/// another event made it at the same time.
pub(crate) fn is_taken(error: &Error) -> bool {
    match error {
        Error::Database(sqlx::Error::Database(db)) => db.constraint() == Some(sources::COMMIT_KEY),
        _ => false,
    }
}

/// Cancels the jobs of a project that wait for an approval on another commit of the pull request
/// of a source, and records their reports. Returns the cancelled jobs.
pub(crate) async fn replace_waiting(
    pool: &PgPool,
    project: ProjectId,
    source: &JobSource,
) -> Result<Vec<JobId>, Error> {
    // Find the jobs that wait on another commit of the pull request.
    let Some(pull) = source.pull_requests.first() else {
        return Ok(Vec::new());
    };
    let mut tx = pool.begin().await?;
    let waiting = sources::waiting_of_pull(
        &mut tx,
        project,
        &source.repository,
        pull.number,
        &source.sha,
    )
    .await?;

    // Cancel each job that still waits, with its row locked, and record its report.
    let short: String = source.sha.chars().take(7).collect();
    let reason = format!("Replaced by the commit {short}");
    let mut cancelled = Vec::new();
    for id in waiting {
        let Some(mut job) = jobs::find(&mut tx, id, true).await? else {
            continue;
        };
        if job.status != JobStatus::AwaitingApproval {
            continue;
        }
        job.end(JobStatus::Cancelled, Some(&reason), now());
        jobs::save(&mut tx, &mut job).await?;
        reports::job_changed(&mut tx, &job).await?;
        cancelled.push(id);
    }
    tx.commit().await?;
    for id in &cancelled {
        tracing::info!(event = "job.cancelled", job = %id);
    }
    Ok(cancelled)
}
