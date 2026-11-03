//! The fire of a trigger: a job of its pipeline with its inputs, as the runner of the triggers
//! of its workspace, and the record of the result on the trigger.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::projects::jobs::Job;
use scylla_domain::projects::pipelines::{EnvValue, EnvVar};
use scylla_domain::projects::triggers::{FireResult, InputValue, Trigger, TriggerId, RUNNER_KIND};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Hooks;
use serde_json::Value;
use sqlx::PgPool;

use super::store::{runners, triggers};
use crate::features::identity::actors;
use crate::features::identity::agents::Registry;
use crate::features::projects::pipelines::service as pipelines;
use crate::Error;

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

/// Fires a trigger by its id, with no body.
pub(crate) async fn by_id(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    id: TriggerId,
) -> Result<Job, FireError> {
    let mut tx = pool.begin().await.map_err(Error::from)?;
    let trigger = triggers::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("trigger"))?;
    tx.commit().await.map_err(Error::from)?;
    fire(pool, hooks, registry, &trigger, None).await
}

/// Fires a trigger with the body of a webhook, if one came, and records the result.
pub(crate) async fn fire(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    trigger: &Trigger,
    body: Option<&Value>,
) -> Result<Job, FireError> {
    let result = start(pool, hooks, registry, trigger, body).await;
    let record = match &result {
        Ok(_) => FireResult::Ok,
        Err(FireError::Payload(reason)) => FireResult::Error(reason.clone()),
        Err(FireError::Failed(error)) => FireResult::Error(error.to_string()),
    };
    let mut tx = pool.begin().await.map_err(Error::from)?;
    triggers::record(&mut tx, trigger.id, &record)
        .await
        .map_err(Error::from)?;
    tx.commit().await.map_err(Error::from)?;
    match &result {
        Ok(job) => tracing::info!(event = "trigger.fired", trigger = %trigger.id, job = %job.id),
        Err(_) => tracing::warn!(event = "trigger.failed", trigger = %trigger.id, ?record),
    }
    result
}

/// Starts the job of a fire.
async fn start(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    trigger: &Trigger,
    body: Option<&Value>,
) -> Result<Job, FireError> {
    let vars = inputs(trigger, body)?;
    let pipeline = pipelines::get(pool, trigger.pipeline_id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    let actor = Actor {
        id: runner(pool, trigger.workspace_id).await?,
        kind: RUNNER_KIND,
    };
    let job = pipelines::start(
        pool,
        hooks,
        registry,
        actor,
        pipeline,
        &vars,
        Some(trigger.id),
    );
    Ok(job.await?)
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
