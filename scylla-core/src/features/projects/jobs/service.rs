//! The logic of the jobs that people call: one transaction for each call, with the hooks. A
//! change of a job that an agent holds also tells its stream.

use scylla_domain::identity::actors::Actor;
use scylla_domain::projects::jobs::{
    Job, JobId, JobPage, JobStatus, ListJobs, LogBatch, LogBatchId, NewJob, KIND,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::{CreateContext, DeleteContext, Hooks, Rejection};
use sqlx::PgPool;

use super::runs::now;
use super::store::{jobs, logs};
use crate::features::identity::agents::Registry;
use crate::Error;

/// Creates a pending job if each `BeforeCreate` hook accepts it, then wakes the agents.
pub(crate) async fn create(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    actor: Actor,
    new: NewJob,
) -> Result<Job, Error> {
    let mut tx = pool.begin().await?;
    hooks
        .before_create(&mut tx, &CreateContext::new(KIND, actor, new.workspace_id))
        .await?;
    let job = jobs::insert(&mut tx, &new).await?;
    tx.commit().await?;
    registry.wake_all();
    tracing::info!(event = "job.created", job = %job.id, pipeline = %job.pipeline_id);
    Ok(job)
}

/// Returns a job, or `None` if no job has this id.
pub(crate) async fn get(pool: &PgPool, id: JobId) -> Result<Option<Job>, Error> {
    let mut tx = pool.begin().await?;
    let job = jobs::find(&mut tx, id, false).await?;
    tx.commit().await?;
    Ok(job)
}

/// Returns the project and the workspace of a job, or `None` if no job has this id.
pub(crate) async fn place(
    pool: &PgPool,
    id: JobId,
) -> Result<Option<(ProjectId, WorkspaceId)>, Error> {
    let mut tx = pool.begin().await?;
    let place = jobs::place(&mut tx, id).await?;
    tx.commit().await?;
    Ok(place)
}

/// Returns one page of jobs, newest first.
pub(crate) async fn list(pool: &PgPool, query: ListJobs) -> Result<JobPage, Error> {
    let mut tx = pool.begin().await?;
    let items = jobs::list(&mut tx, query.scope, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(items, query.page.size, |job| job.id))
}

/// Cancels a job that did not end, and tells its agent. A job that ended does not change.
pub(crate) async fn cancel(pool: &PgPool, registry: &Registry, id: JobId) -> Result<Job, Error> {
    // End the job in one transaction, with the row locked.
    let mut tx = pool.begin().await?;
    let mut job = jobs::find(&mut tx, id, true)
        .await?
        .ok_or(Error::NotFound("job"))?;
    let changed = job.end(JobStatus::Cancelled, None, now());
    if changed {
        jobs::save(&mut tx, &mut job).await?;
    }
    tx.commit().await?;

    // Tell the agent, after the commit.
    if let (true, Some(agent)) = (changed, job.agent_id) {
        registry.cancel(agent, id);
        registry.wake_all();
    }
    if changed {
        tracing::info!(event = "job.cancelled", job = %id);
    }
    Ok(job)
}

/// Deletes a job with its logs, runs the `AfterDelete` hooks, then tells its agent.
pub(crate) async fn delete(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    actor: Actor,
    id: JobId,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    let (workspace, agent, status) = jobs::delete(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("job"))?;
    hooks
        .after_delete(&mut tx, &DeleteContext::new(KIND, actor, workspace))
        .await?;
    tx.commit().await?;
    if let (false, Some(agent)) = (status.is_final(), agent) {
        registry.cancel(agent, id);
        registry.wake_all();
    }
    tracing::info!(event = "job.deleted", job = %id);
    Ok(())
}

/// Starts a new pending job with the nodes and the pipeline of a job that ended.
pub(crate) async fn rerun(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    actor: Actor,
    id: JobId,
) -> Result<Job, Error> {
    let old = get(pool, id).await?.ok_or(Error::NotFound("job"))?;
    if !old.status.is_final() {
        let message = "only a job that ended runs again";
        return Err(Error::Rejected(Rejection::new("JOB_NOT_ENDED", message)));
    }
    let new = NewJob {
        pipeline_id: old.pipeline_id,
        project_id: old.project_id,
        workspace_id: old.workspace_id,
        created_by: actor.id,
        rerun_of: Some(old.id),
        nodes: old.nodes,
    };
    create(pool, hooks, registry, actor, new).await
}

/// Returns at most `limit` batches of the logs of a job after a cursor, of one node if it is
/// set.
pub(crate) async fn logs(
    pool: &PgPool,
    id: JobId,
    node: Option<&str>,
    cursor: Option<LogBatchId>,
    limit: i64,
) -> Result<Vec<LogBatch>, Error> {
    let mut tx = pool.begin().await?;
    let batches = logs::after(&mut tx, id, node, cursor, limit).await?;
    tx.commit().await?;
    Ok(batches)
}
