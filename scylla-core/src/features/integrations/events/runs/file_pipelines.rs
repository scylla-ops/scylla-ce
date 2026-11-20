//! The runs of a Git event in a project of files: each file pipeline whose file at the commit of
//! the event matches it gets one run for the commit, as the runner of the triggers. A file with
//! errors gets a failed run with no node on each push and pull request.

use scylla_domain::identity::actors::Actor;
use scylla_domain::integrations::sources::JobSource;
use scylla_domain::projects::git_rules::TriggerEvent;
use scylla_domain::projects::jobs::StartJob;
use scylla_domain::projects::triggers::RUNNER_KIND;

use crate::features::integrations::links::store::Linked;
use crate::features::integrations::{files, sources, IntegrationContext};
use crate::features::projects::jobs::commits;
use crate::features::projects::pipelines::service as pipelines;
use crate::features::projects::triggers::fire;
use crate::Error;

/// Starts the runs of the file pipelines of a Git event for one active link of a project of
/// files. A push to the default branch first syncs the file pipelines with its head.
pub(super) async fn handle(
    ctx: &IntegrationContext,
    linked: &Linked,
    event: &TriggerEvent<'_>,
    source: JobSource,
) -> Result<(), Error> {
    // Sync the file pipelines on a push to the default branch.
    let project = linked.link.project_id;
    let default = &linked.link.default_branch;
    if matches!(event, TriggerEvent::Push { branch, .. } if *branch == default.as_str()) {
        files::sync_default_branch(ctx, project, Some(&source.sha)).await?;
    }

    // Find the file pipelines that the event starts, from the files at its commit.
    let mut starts = files::starts_of_event(ctx, linked, event, &source).await?;
    if starts.is_empty() {
        return Ok(());
    }

    // Read the files of a pull request from a fork.
    if let (true, Some(pull)) = (source.from_fork(), source.pull_requests.first()) {
        let changes = sources::changes_pipeline_files(ctx, project, pull.number).await;
        for own in starts.iter_mut().filter_map(|start| start.source.as_mut()) {
            own.changes_pipeline_files = changes;
        }
    }

    // Start or join the run of each pipeline, as the runner of the triggers.
    let runner = Actor {
        id: fire::runner(&ctx.pool, linked.link.workspace_id).await?,
        kind: RUNNER_KIND,
    };
    let mut first = Ok(());
    for start in starts {
        let done = run(ctx, &runner, start, &source).await;
        first = first.and(done);
    }
    first
}

/// Joins the run of the pipeline of a start on the commit of a source, or starts it. Another
/// event that starts the run of the commit at the same time wins, and this one joins it.
async fn run(
    ctx: &IntegrationContext,
    runner: &Actor,
    start: StartJob,
    source: &JobSource,
) -> Result<(), Error> {
    // Join the run of the commit, if the pipeline has one.
    let pipeline = start.pipeline_id;
    if commits::join(&ctx.pool, pipeline, source).await?.is_some() {
        return Ok(());
    }

    // Start the run, or join the run that another event started at the same time.
    let (pool, hooks, registry) = (&ctx.pool, &ctx.hooks, &ctx.registry);
    let started = pipelines::start(pool, hooks, registry, runner.clone(), start).await;
    if started.as_ref().is_err_and(commits::is_taken) {
        commits::join(&ctx.pool, pipeline, source).await?;
        return Ok(());
    }
    started.map(|_| ())
}
