//! The logic of the jobs that the agents call on their stream: the placement of a job with the
//! values of its secrets, the events, the logs of the nodes and of the checkout, and the check of
//! the jobs after a new stream. Each change of the state of a job records it for the reports, in
//! its transaction.

use std::collections::HashMap;

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{CheckoutRun, Job, JobEvent, JobId, JobStatus, LogLine};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::DomainError;
use sqlx::PgPool;

use super::env::{self, ResolvedEnv};
use super::store::{holds, jobs, logs};
use crate::features::integrations::reports;
use crate::infra::secret::Sealer;
use crate::Error;

/// A job for an agent: the job, and the environment of each node with the values.
pub(crate) struct Assignment {
    /// The job.
    pub(crate) job: Job,
    /// The environment of each node, in the order of the nodes.
    pub(crate) env: Vec<Vec<ResolvedEnv>>,
}

/// What the stream does after a message of the agent.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    /// Nothing.
    Nothing,
    /// Tell the agent to stop the job.
    Cancel,
    /// The job ended: the agent can take another job.
    Ended,
}

/// Returns the number of jobs that an agent holds and that did not end.
pub(crate) async fn held(pool: &PgPool, agent: AppId) -> Result<usize, Error> {
    let mut tx = pool.begin().await?;
    let held = holds::held_by(&mut tx, agent).await?;
    tx.commit().await?;
    Ok(held.len())
}

/// Returns the number of jobs that each agent of `agents` holds and that did not end. An agent
/// with no job is not in the map.
pub(crate) async fn held_counts(
    pool: &PgPool,
    agents: &[AppId],
) -> Result<HashMap<AppId, u32>, Error> {
    let mut tx = pool.begin().await?;
    let counts = holds::counts(&mut tx, agents).await?;
    tx.commit().await?;
    Ok(counts)
}

/// Gives the oldest pending job of a workspace to an agent, with the values of its secrets: of
/// each project, or of `projects` only. A job with a checkout goes only to an agent with
/// `checkout`. A job whose secrets do not open fails with the reason, and the next job comes.
pub(crate) async fn claim(
    pool: &PgPool,
    sealer: Option<&Sealer>,
    workspace: WorkspaceId,
    projects: Option<&[ProjectId]>,
    agent: AppId,
    containers: bool,
    checkout: bool,
) -> Result<Option<Assignment>, Error> {
    loop {
        // Take the oldest free job.
        let mut tx = pool.begin().await?;
        let job = jobs::claim(&mut tx, workspace, projects, agent, containers, checkout).await?;
        if let Some(job) = &job {
            reports::job_changed(&mut tx, job).await?;
        }
        tx.commit().await?;
        let Some(job) = job else {
            return Ok(None);
        };

        // Open its secrets, or fail it.
        match env::resolve(pool, sealer, &job).await {
            Ok(env) => return Ok(Some(Assignment { job, env })),
            Err(reason) => fail(pool, job.id, &reason).await?,
        }
    }
}

/// Fails a job that did not end, with a reason.
pub(crate) async fn fail(pool: &PgPool, id: JobId, reason: &str) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if let Some(mut job) = jobs::find(&mut tx, id, true).await? {
        if job.end(JobStatus::Failed, Some(reason), now()) {
            jobs::save(&mut tx, &mut job).await?;
            reports::job_changed(&mut tx, &job).await?;
        }
    }
    tx.commit().await?;
    tracing::warn!(event = "job.failed", job = %id, reason, "the dispatch failed");
    Ok(())
}

/// Applies an event of an agent to a job that it holds. An event of a job that the agent does
/// not hold, or of a job that ended, gives `Cancel`. An event that does not fit the state of the
/// job changes nothing.
pub(crate) async fn apply(
    pool: &PgPool,
    agent: AppId,
    id: JobId,
    event: &JobEvent,
) -> Result<Reply, Error> {
    // Lock the job, and refuse a job of another agent or a job that ended.
    let mut tx = pool.begin().await?;
    let Some(mut job) = jobs::find(&mut tx, id, true).await? else {
        return Ok(Reply::Cancel);
    };
    if job.agent_id != Some(agent) || job.status.is_final() {
        return Ok(Reply::Cancel);
    }

    // Apply the event, and save the new state.
    match job.apply(event, now()) {
        Ok(()) => {}
        Err(DomainError::NotAllowed) => {
            tracing::warn!(job = %id, ?event, "an event does not fit the state of the job");
            return Ok(Reply::Nothing);
        }
        Err(error) => return Err(Error::Internal(error.to_string())),
    }
    jobs::save(&mut tx, &mut job).await?;
    reports::job_changed(&mut tx, &job).await?;
    tx.commit().await?;
    if job.status.is_final() {
        let status = job.status.as_str();
        tracing::info!(event = "job.ended", job = %id, status);
        return Ok(Reply::Ended);
    }
    Ok(Reply::Nothing)
}

/// Stores a batch of lines of a node of a job that the agent holds, or of its checkout. A job of
/// another agent, or a node that the job does not have, gives `Cancel`.
pub(crate) async fn append(
    pool: &PgPool,
    agent: AppId,
    id: JobId,
    node: &str,
    lines: &[LogLine],
) -> Result<Reply, Error> {
    let mut tx = pool.begin().await?;
    let Some(job) = jobs::find(&mut tx, id, false).await? else {
        return Ok(Reply::Cancel);
    };
    let known = (node == CheckoutRun::NODE && job.checkout.is_some())
        || job
            .nodes
            .iter()
            .any(|candidate| candidate.id.as_str() == node);
    if job.agent_id != Some(agent) || !known {
        return Ok(Reply::Cancel);
    }
    logs::insert(&mut tx, id, node, lines).await?;
    tx.commit().await?;
    Ok(Reply::Nothing)
}

/// Checks the jobs of an agent after a new stream: a job that it holds and does not run ends as
/// orphaned if it ran, or goes back to the pool if it waited. Returns the jobs that it runs and
/// must stop, and `true` if a job went back to the pool.
pub(crate) async fn reconcile(
    pool: &PgPool,
    agent: AppId,
    running: &[JobId],
) -> Result<(Vec<JobId>, bool), Error> {
    let mut tx = pool.begin().await?;
    let held = holds::held_by(&mut tx, agent).await?;
    let mut released = false;
    for id in held.iter().filter(|id| !running.contains(id)) {
        released |= release(
            &mut tx,
            *id,
            "the agent does not run the job after a new stream",
        )
        .await?;
    }
    tx.commit().await?;
    let stop = running
        .iter()
        .filter(|id| !held.contains(id))
        .copied()
        .collect();
    Ok((stop, released))
}

/// Releases each job that an agent holds, in the transaction of the caller. Returns `true` if a
/// job went back to the pool.
pub(crate) async fn release_all(
    tx: &mut scylla_extension::Tx<'_>,
    agent: AppId,
    reason: &str,
) -> Result<bool, Error> {
    let mut pooled = false;
    for id in holds::held_by(tx, agent).await? {
        pooled |= release(tx, id, reason).await?;
    }
    Ok(pooled)
}

/// Ends a running job as orphaned, or gives a pending job back to the pool. Returns `true` if
/// the job went back to the pool.
pub(crate) async fn release(
    tx: &mut scylla_extension::Tx<'_>,
    id: JobId,
    reason: &str,
) -> Result<bool, Error> {
    let Some(mut job) = jobs::find(tx, id, true).await? else {
        return Ok(false);
    };
    let pooled = match job.status {
        JobStatus::Pending => {
            job.agent_id = None;
            true
        }
        JobStatus::Running => {
            job.end(JobStatus::Orphaned, Some(reason), now());
            tracing::warn!(event = "job.orphaned", job = %id, reason);
            false
        }
        _ => return Ok(false),
    };
    jobs::save(tx, &mut job).await?;
    reports::job_changed(tx, &job).await?;
    Ok(pooled)
}

/// Returns the jobs of a pipeline that an agent holds and that did not end, with their agent, in
/// the transaction of the caller.
pub(crate) async fn held_in_pipeline(
    tx: &mut scylla_extension::Tx<'_>,
    pipeline: scylla_domain::projects::pipelines::PipelineId,
) -> Result<Vec<(JobId, AppId)>, Error> {
    Ok(holds::held_in_pipeline(tx, pipeline).await?)
}

/// Returns the time now, in whole microseconds as the database keeps it.
pub(crate) fn now() -> time::OffsetDateTime {
    let now = time::OffsetDateTime::now_utc();
    now.replace_nanosecond(now.nanosecond() / 1_000 * 1_000)
        .unwrap_or(now)
}
