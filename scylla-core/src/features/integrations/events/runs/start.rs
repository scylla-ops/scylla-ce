//! The runs of a Git event for one linked project: each editor pipeline whose enabled Git trigger
//! matches the event gets one run for the commit, as the runner of the triggers of the workspace.
//! An event on a commit that has a run of the pipeline adds its branch or its pull request to the
//! source of that run.

use std::collections::HashSet;

use scylla_domain::identity::actors::Actor;
use scylla_domain::integrations::events::GitEvent;
use scylla_domain::integrations::links::PipelineSource;
use scylla_domain::integrations::sources::JobSource;
use scylla_domain::projects::git_rules::TriggerEvent;
use scylla_domain::projects::jobs::{Job, StartJob};
use scylla_domain::projects::pipelines::{EnvValue, EnvVar};
use scylla_domain::projects::triggers::{FireResult, Input, InputValue, RUNNER_KIND};

use super::file_pipelines;
use super::store::{self, GitTrigger};
use crate::features::integrations::links::store::Linked;
use crate::features::integrations::{sources, IntegrationContext};
use crate::features::projects::jobs::commits;
use crate::features::projects::pipelines::service as pipelines;
use crate::features::projects::triggers::fire;
use crate::Error;

/// Starts the runs of a Git event for one active link: of the editor pipelines, or of the file
/// pipelines in a project of files. A pull request from a fork first cancels the runs of its older
/// commits that wait for an approval. A closed pull request starts nothing.
pub(crate) async fn handle(
    ctx: &IntegrationContext,
    linked: &Linked,
    event: &GitEvent,
) -> Result<(), Error> {
    // Take only an event that can start a run.
    let (Some(checked), Some(mut source)) = (event.trigger(), event.source()) else {
        return Ok(());
    };
    let project = linked.link.project_id;

    // Cancel the runs of the older commits of a pull request from a fork that wait.
    if source.from_fork() {
        commits::replace_waiting(&ctx.pool, project, &source).await?;
    }

    // Start the file pipelines of a project of files.
    if linked.link.pipeline_source == PipelineSource::File {
        return file_pipelines::handle(ctx, linked, &checked, source).await;
    }

    // Find the first trigger of each pipeline that the event starts.
    let mut tx = ctx.pool.begin().await?;
    let triggers = store::git_triggers(&mut tx, project).await?;
    tx.commit().await?;
    let matched = first_of_each_pipeline(triggers, &checked);
    if matched.is_empty() {
        return Ok(());
    }

    // Read the files of a pull request from a fork.
    if let (true, Some(pull)) = (source.from_fork(), source.pull_requests.first()) {
        let number = pull.number;
        source.changes_pipeline_files = sources::changes_pipeline_files(ctx, project, number).await;
    }

    // Start or join the run of each pipeline, as the runner of the triggers.
    let runner = Actor {
        id: fire::runner(&ctx.pool, linked.link.workspace_id).await?,
        kind: RUNNER_KIND,
    };
    let mut first = Ok(());
    for trigger in &matched {
        let done = run(ctx, &runner, trigger, &source).await;
        first = first.and(done);
    }
    first
}

/// Keeps the triggers whose rules match an event, the first one of each pipeline.
fn first_of_each_pipeline(triggers: Vec<GitTrigger>, event: &TriggerEvent<'_>) -> Vec<GitTrigger> {
    let mut pipelines = HashSet::new();
    triggers
        .into_iter()
        .filter(|trigger| trigger.rules.matches(event))
        .filter(|trigger| pipelines.insert(trigger.pipeline_id))
        .collect()
}

/// Joins the run of the pipeline of a trigger on the commit of a source, or starts it and records
/// the fire on the trigger. Another event that starts the run of the commit at the same time wins,
/// and this one joins it.
async fn run(
    ctx: &IntegrationContext,
    runner: &Actor,
    trigger: &GitTrigger,
    source: &JobSource,
) -> Result<(), Error> {
    // Join the run of the commit, if the pipeline has one.
    if commits::join(&ctx.pool, trigger.pipeline_id, source)
        .await?
        .is_some()
    {
        return Ok(());
    }

    // Start the run, or join the run that another event started at the same time.
    let started = start(ctx, runner, trigger, source).await;
    if started.as_ref().is_err_and(commits::is_taken) {
        commits::join(&ctx.pool, trigger.pipeline_id, source).await?;
        return Ok(());
    }

    // Record the fire on the trigger.
    let result = match &started {
        Ok(_) => FireResult::Ok,
        Err(error) => FireResult::Error(error.to_string()),
    };
    let mut tx = ctx.pool.begin().await?;
    store::record(&mut tx, trigger.id, &result).await?;
    tx.commit().await?;
    match &started {
        Ok(job) => tracing::info!(event = "trigger.fired", trigger = %trigger.id, job = %job.id),
        Err(_) => tracing::warn!(event = "trigger.failed", trigger = %trigger.id, ?result),
    }
    started.map(|_| ())
}

/// Starts the run of the pipeline of a trigger on the commit of a source, with the inputs of the
/// trigger.
async fn start(
    ctx: &IntegrationContext,
    runner: &Actor,
    trigger: &GitTrigger,
    source: &JobSource,
) -> Result<Job, Error> {
    let pipeline = pipelines::get(&ctx.pool, trigger.pipeline_id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    let start = StartJob {
        variables: variables(&trigger.inputs),
        trigger_id: Some(trigger.id),
        source: Some(source.clone()),
        ..StartJob::of(pipeline)
    };
    let (pool, hooks, registry) = (&ctx.pool, &ctx.hooks, &ctx.registry);
    pipelines::start(pool, hooks, registry, runner.clone(), start).await
}

/// Returns the variables of the inputs of a trigger. A Git event has no body, so a pointer gives
/// an empty text.
fn variables(inputs: &[Input]) -> Vec<EnvVar> {
    inputs
        .iter()
        .map(|input| EnvVar {
            name: input.name.clone(),
            value: EnvValue::Literal(match &input.value {
                InputValue::Literal(text) => text.clone(),
                InputValue::Pointer(_) => String::new(),
            }),
        })
        .collect()
}
