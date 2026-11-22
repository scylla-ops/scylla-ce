//! The fire of a trigger: a job of its pipeline with its inputs, as the runner of the triggers
//! of its workspace, and the record of the result on the trigger, in the history of its fires
//! and, for a failure, in the activity feed. A file pipeline runs its file at the head of the
//! default branch, and a Git trigger runs its pipeline on that head. A delivery of a webhook that
//! its conditions skip makes no job: only its record.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::integrations::sources::SourceEvent;
use scylla_domain::projects::jobs::{Job, JobOrigin, StartJob};
use scylla_domain::projects::pipelines::{EnvValue, EnvVar};
use scylla_domain::projects::triggers::{
    FireResult, FireSource, InputValue, Trigger, TriggerId, TriggerKind, RUNNER_KIND,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use serde_json::Value;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::store::fires::{self, NewFire};
use super::store::{runners, triggers};
use crate::features::activity;
use crate::features::identity::actors;
use crate::features::integrations::{files, IntegrationContext};
use crate::features::projects::jobs::runs;
use crate::features::projects::pipelines::service as pipelines;
use crate::Error;

/// What started a fire, and when it was due.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Call {
    /// What started the fire.
    pub(crate) source: FireSource,
    /// The time when the fire was due: the cron time of a schedule, else the time of the request.
    pub(crate) due_at: OffsetDateTime,
}

impl Call {
    /// Returns a call from a source that is due now.
    pub(crate) fn now(source: FireSource) -> Self {
        Self {
            source,
            due_at: runs::now(),
        }
    }
}

/// Why a fire fails.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FireError {
    /// The body has no value at a JSON pointer of an input.
    #[error("{0}")]
    Payload(String),
    /// The job could not start.
    #[error(transparent)]
    Failed(Error),
}

impl From<Error> for FireError {
    fn from(error: Error) -> Self {
        Self::Failed(error)
    }
}

/// Fires a trigger by its id, with no body, for an origin: its time or a caller.
pub(crate) async fn by_id(
    vcs: &IntegrationContext,
    id: TriggerId,
    origin: JobOrigin,
    call: Call,
) -> Result<Job, FireError> {
    let mut tx = vcs.pool.begin().await.map_err(Error::from)?;
    let trigger = triggers::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("trigger"))?;
    tx.commit().await.map_err(Error::from)?;
    fire(vcs, &trigger, None, origin, call).await
}

/// Fires a trigger for an origin with the body of a webhook, if one came, and records the result
/// on the trigger and in the history of its fires.
pub(crate) async fn fire(
    vcs: &IntegrationContext,
    trigger: &Trigger,
    body: Option<&Value>,
    origin: JobOrigin,
    call: Call,
) -> Result<Job, FireError> {
    // Start the job.
    let fired_at = runs::now();
    let result = start(vcs, trigger, body, origin).await;
    let record = match &result {
        Ok(_) => FireResult::Ok,
        Err(FireError::Payload(reason)) => FireResult::Error(reason.clone()),
        Err(FireError::Failed(error)) => FireResult::Error(error.to_string()),
    };

    // Record the result on the trigger and in the history.
    let fire = NewFire {
        trigger: trigger.id,
        source: call.source,
        due_at: call.due_at,
        fired_at,
        result: &record,
        job: result.as_ref().ok().map(|job| job.id),
    };
    let mut tx = vcs.pool.begin().await.map_err(Error::from)?;
    triggers::record(&mut tx, trigger.id, &record)
        .await
        .map_err(Error::from)?;
    fires::insert(&mut tx, fire).await.map_err(Error::from)?;
    if let FireResult::Error(reason) = &record {
        activity::fire_failed(&mut tx, trigger, reason).await?;
    }
    tx.commit().await.map_err(Error::from)?;
    match &result {
        Ok(job) => tracing::info!(event = "trigger.fired", trigger = %trigger.id, job = %job.id),
        Err(_) => tracing::warn!(event = "trigger.failed", trigger = %trigger.id, ?record),
    }
    result
}

/// Records a delivery of a webhook that the conditions of the trigger skip: no job, and the
/// reason on the trigger and in the history of its fires. It is not a failure, so the activity
/// feed does not get it.
pub(crate) async fn skip(
    pool: &PgPool,
    trigger: &Trigger,
    reason: &str,
    call: Call,
) -> Result<(), Error> {
    let record = FireResult::Skipped(reason.to_owned());
    let fire = NewFire {
        trigger: trigger.id,
        source: call.source,
        due_at: call.due_at,
        fired_at: runs::now(),
        result: &record,
        job: None,
    };
    let mut tx = pool.begin().await?;
    triggers::record(&mut tx, trigger.id, &record).await?;
    fires::insert(&mut tx, fire).await?;
    tx.commit().await?;
    tracing::info!(event = "trigger.skipped", trigger = %trigger.id);
    Ok(())
}

/// Starts the job of a fire. A file pipeline takes its file at the head of the default branch,
/// and a Git trigger runs its pipeline on that head.
async fn start(
    vcs: &IntegrationContext,
    trigger: &Trigger,
    body: Option<&Value>,
    origin: JobOrigin,
) -> Result<Job, FireError> {
    // Read the inputs, the pipeline and the runner of the triggers.
    let vars = inputs(trigger, body)?;
    let pipeline = pipelines::get(&vcs.pool, trigger.pipeline_id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    let actor = Actor {
        id: runner(&vcs.pool, trigger.workspace_id).await?,
        kind: RUNNER_KIND,
    };

    // Take the commit at the head of the default branch, when the pipeline or the trigger needs it.
    let git = matches!(
        trigger.kind,
        TriggerKind::Push(_) | TriggerKind::PullRequest(_) | TriggerKind::Tag(_)
    );
    let start = if pipeline.file.is_some() {
        files::start_at(vcs, pipeline, None, SourceEvent::Trigger, origin).await?
    } else if git {
        let project = pipeline.project_id;
        let source = files::head_source(vcs, project, SourceEvent::Trigger).await?;
        StartJob {
            source: Some(source),
            ..StartJob::of(pipeline, origin)
        }
    } else {
        StartJob::of(pipeline, origin)
    };

    // Start the job with the inputs, as the runner.
    let start = StartJob {
        variables: vars,
        trigger_id: Some(trigger.id),
        ..start
    };
    let (pool, hooks, registry) = (&vcs.pool, &vcs.hooks, &vcs.registry);
    Ok(pipelines::start(pool, hooks, registry, actor, start).await?)
}

/// Returns the variables of the inputs of a trigger. A pointer reads the body: a text gives the
/// text, another value its JSON, and no body an empty text.
fn inputs(trigger: &Trigger, body: Option<&Value>) -> Result<Vec<EnvVar>, FireError> {
    trigger
        .inputs
        .iter()
        .map(|input| {
            let value = match (&input.value, body) {
                (InputValue::Literal(text), _) => text.clone(),
                (InputValue::Pointer(_), None) => String::new(),
                (InputValue::Pointer(pointer), Some(body)) => match body.pointer(pointer) {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Null) => String::new(),
                    Some(other) => other.to_string(),
                    None => {
                        let reason =
                            format!("the body has no value at {pointer} for {}", input.name);
                        return Err(FireError::Payload(reason));
                    }
                },
            };
            Ok(EnvVar {
                name: input.name.clone(),
                value: EnvValue::Literal(value),
            })
        })
        .collect()
}

/// Returns the runner of the triggers of a workspace, and makes it at the first fire.
pub(crate) async fn runner(pool: &PgPool, workspace: WorkspaceId) -> Result<ActorId, Error> {
    // Return the runner, if the workspace has one.
    let mut tx = pool.begin().await?;
    if let Some(actor) = runners::find(&mut tx, workspace).await? {
        return Ok(actor);
    }

    // Make it. Another server can make it at the same time: then read the one of the other.
    let actor = actors::insert(&mut tx, RUNNER_KIND).await?;
    if runners::insert(&mut tx, workspace, actor.id).await? {
        tx.commit().await?;
        return Ok(actor.id);
    }
    tx.rollback().await?;
    let mut tx = pool.begin().await?;
    let actor = runners::find(&mut tx, workspace)
        .await?
        .ok_or(Error::NotFound("runner"))?;
    tx.commit().await?;
    Ok(actor)
}
