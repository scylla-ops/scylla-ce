//! The logic of the pipelines: one transaction for each call, with the hooks.

use scylla_domain::identity::actors::Actor;
use scylla_domain::integrations::sources::{GitRef, SourceEvent};
use scylla_domain::projects::jobs::{Job, StartJob};
use scylla_domain::projects::pipelines::{
    ListPipelines, NewPipeline, Node, Pipeline, PipelineChange, PipelineId, PipelineLayout,
    PipelinePage, PipelineScope, KIND,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::{CreateContext, DeleteContext, Hooks, Rejection, Tx};
use sqlx::PgPool;

use super::store::{self, Changed};
use crate::features::identity::agents::Registry;
use crate::features::integrations::{files, sources, IntegrationContext};
use crate::features::projects::jobs::{self, runs};
use crate::features::projects::{secrets, service as projects};
use crate::Error;

/// The largest JSON of the nodes of a pipeline, in bytes.
const MAX_NODES_JSON: usize = 8 << 20;

/// The code of a change of a pipeline that is read only.
const READ_ONLY: &str = "PIPELINE_READ_ONLY";

/// Creates a pipeline in a project if each `BeforeCreate` hook accepts it.
pub(crate) async fn create(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    workspace: WorkspaceId,
    new: NewPipeline,
) -> Result<Pipeline, Error> {
    // Check the size of the nodes, refuse a project of files, then run the hooks.
    check_size(&new.nodes)?;
    let mut tx = pool.begin().await?;
    if store::uses_files(&mut tx, new.project_id).await? {
        let message = "the project takes its pipelines from the files of its repository";
        return Err(Error::Rejected(Rejection::new(READ_ONLY, message)));
    }
    hooks
        .before_create(&mut tx, &CreateContext::new(KIND, actor, workspace))
        .await?;

    // Insert the pipeline with the places of its nodes.
    let pipeline = store::insert(&mut tx, &new)
        .await?
        .ok_or(Error::NotFound("project"))?;
    save_places(&mut tx, pipeline.id, new.layout, &ids_of(&pipeline.nodes)).await?;
    tx.commit().await?;
    Ok(pipeline)
}

/// Returns the workspace of a project, or `None` if no project has this id.
pub(crate) async fn project_workspace(
    pool: &PgPool,
    id: ProjectId,
) -> Result<Option<WorkspaceId>, Error> {
    projects::workspace(pool, id).await
}

/// Returns a pipeline, or `None` if no pipeline has this id.
pub(crate) async fn get(pool: &PgPool, id: PipelineId) -> Result<Option<Pipeline>, Error> {
    let mut tx = pool.begin().await?;
    let pipeline = store::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(pipeline)
}

/// Returns a pipeline with its saved places, or `None` if no pipeline has this id. An active file
/// pipeline first reads the default branch again when it changed, at most each 30 seconds.
pub(crate) async fn get_with_layout(
    ctx: &IntegrationContext,
    id: PipelineId,
) -> Result<Option<(Pipeline, Option<PipelineLayout>)>, Error> {
    // Read the default branch again for an active file pipeline.
    let pipeline = get(&ctx.pool, id).await?;
    if let Some(pipeline) = pipeline.as_ref().filter(|found| found.file.is_some()) {
        files::refresh(ctx, pipeline.project_id).await;
    }

    // Read the pipeline and its places.
    let mut tx = ctx.pool.begin().await?;
    let pipeline = store::find(&mut tx, id).await?;
    let layout = store::find_layout(&mut tx, id).await?;
    tx.commit().await?;
    Ok(pipeline.map(|pipeline| (pipeline, layout)))
}

/// Saves places of a pipeline. The version of the pipeline does not change.
pub(crate) async fn save_layout(
    pool: &PgPool,
    id: PipelineId,
    layout: PipelineLayout,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    let nodes = store::lock_node_ids(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    save_places(&mut tx, id, layout, &nodes).await?;
    tx.commit().await?;
    Ok(())
}

/// Saves the places of the named nodes, and ignores the other places.
async fn save_places(
    tx: &mut Tx<'_>,
    id: PipelineId,
    mut layout: PipelineLayout,
    nodes: &[String],
) -> Result<(), sqlx::Error> {
    layout
        .nodes
        .retain(|node, _| nodes.iter().any(|name| name == node.as_str()));
    if layout.is_empty() {
        return Ok(());
    }
    store::save_layout(tx, id, &layout).await
}

/// Returns the ids of nodes as text.
fn ids_of(nodes: &[Node]) -> Vec<String> {
    nodes
        .iter()
        .map(|node| node.id.as_str().to_owned())
        .collect()
}

/// Returns the project and the workspace of a pipeline, or `None` if no pipeline has this id.
pub(crate) async fn place(
    pool: &PgPool,
    id: PipelineId,
) -> Result<Option<(ProjectId, WorkspaceId)>, Error> {
    let mut tx = pool.begin().await?;
    let place = store::place(&mut tx, id).await?;
    tx.commit().await?;
    Ok(place)
}

/// Returns one page of pipelines, oldest first. A project of files first reads its default
/// branch again when it changed, at most each 30 seconds.
pub(crate) async fn list(
    ctx: &IntegrationContext,
    query: ListPipelines,
) -> Result<PipelinePage, Error> {
    if let (PipelineScope::Project(project), None) = (query.scope, query.page.after) {
        files::refresh(ctx, project).await;
    }
    let mut tx = ctx.pool.begin().await?;
    let items = store::list(&mut tx, query.scope, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(items, query.page.size, |pipeline| {
        pipeline.id
    }))
}

/// Changes a pipeline that still has the version of the change. Another version gives
/// `ABORTED`. The places of the nodes that the change removes go away.
pub(crate) async fn update(
    pool: &PgPool,
    id: PipelineId,
    change: PipelineChange,
) -> Result<Pipeline, Error> {
    // Check the size of the new nodes.
    if let Some(nodes) = &change.nodes {
        check_size(nodes)?;
    }

    // Refuse a read only pipeline, then change the pipeline and its places.
    let mut tx = pool.begin().await?;
    refuse_read_only(&mut tx, id).await?;
    let changed = store::update(&mut tx, id, &change).await?;
    if let Changed::Done(pipeline) = &changed {
        let nodes = ids_of(&pipeline.nodes);
        if change.nodes.is_some() {
            store::keep_places(&mut tx, id, &nodes).await?;
        }
        save_places(&mut tx, id, change.layout, &nodes).await?;
    }
    tx.commit().await?;

    // Answer the result of the change.
    match changed {
        Changed::Done(pipeline) => Ok(*pipeline),
        Changed::Stale => Err(Error::Conflict("the pipeline changed: read it again")),
        Changed::Missing => Err(Error::NotFound("pipeline")),
    }
}

/// Deletes a pipeline with its jobs and their logs, runs the `AfterDelete` hooks, then tells
/// the agents of its running jobs.
pub(crate) async fn delete(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    actor: Actor,
    id: PipelineId,
) -> Result<(), Error> {
    // Refuse a read only pipeline, then delete the row and run the hooks in one transaction.
    let mut tx = pool.begin().await?;
    refuse_read_only(&mut tx, id).await?;
    let (_, workspace) = store::place(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    let held = runs::held_in_pipeline(&mut tx, id).await?;
    store::delete(&mut tx, id).await?;
    hooks
        .after_delete(&mut tx, &DeleteContext::new(KIND, actor, workspace))
        .await?;
    tx.commit().await?;

    // Tell the agents after the commit.
    for (job, agent) in &held {
        registry.cancel(*agent, *job);
    }
    if !held.is_empty() {
        registry.wake_all();
    }
    tracing::info!(event = "pipeline.deleted", pipeline = %id);
    Ok(())
}

/// Starts a job of a pipeline with a snapshot of its nodes, with no source, or on a commit of the
/// linked repository. A file pipeline takes the file of its key at the ref, or at the head of the
/// default branch with no ref. A secret that a node names and that the project does not have
/// gives `NOT_FOUND`.
pub(crate) async fn run(
    ctx: &IntegrationContext,
    actor: Actor,
    id: PipelineId,
    git_ref: Option<GitRef>,
) -> Result<Job, Error> {
    let pipeline = get(&ctx.pool, id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    let start = match (&pipeline.file, &git_ref) {
        (Some(_), _) => files::start_at(ctx, pipeline, git_ref, SourceEvent::Manual).await?,
        (None, Some(git_ref)) => StartJob {
            source: Some(sources::resolve(ctx, pipeline.project_id, git_ref).await?),
            ..StartJob::of(pipeline)
        },
        (None, None) => StartJob::of(pipeline),
    };
    self::start(&ctx.pool, &ctx.hooks, &ctx.registry, actor, start).await
}

/// Starts a job: a run, a fire of a trigger or a new run of a job. Each node gets the variables
/// that it does not have, and a job from a fork waits for an approval. A secret that a node names
/// and that the project does not have gives `NOT_FOUND`.
pub(crate) async fn start(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    actor: Actor,
    start: StartJob,
) -> Result<Job, Error> {
    if let Some(name) = secrets::missing(pool, start.project_id, &start.secrets()).await? {
        let pipeline = start.pipeline_id;
        tracing::info!(%pipeline, secret = name, "a pipeline names a missing secret");
        return Err(Error::NotFound("secret"));
    }
    let new = start.into_new(actor.id);
    jobs::create(pool, hooks, registry, actor, new).await
}

/// Locks a pipeline, and refuses a file pipeline or an archived pipeline with
/// `PIPELINE_READ_ONLY`.
async fn refuse_read_only(tx: &mut Tx<'_>, id: PipelineId) -> Result<(), Error> {
    match store::lock_read_only(tx, id).await? {
        Some(true) => {
            let message = "the pipeline comes from a file or is archived: it is read only";
            Err(Error::Rejected(Rejection::new(READ_ONLY, message)))
        }
        Some(false) => Ok(()),
        None => Err(Error::NotFound("pipeline")),
    }
}

/// Refuses nodes whose JSON is larger than 8 MiB.
fn check_size(nodes: &scylla_domain::projects::pipelines::Nodes) -> Result<(), Error> {
    let size = super::json::nodes(nodes.as_slice()).to_string().len();
    if size > MAX_NODES_JSON {
        return Err(Error::invalid("nodes", "must be at most 8 MiB in JSON"));
    }
    Ok(())
}
