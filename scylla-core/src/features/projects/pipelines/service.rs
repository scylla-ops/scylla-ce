//! The logic of the pipelines: one transaction for each call, with the hooks.

use scylla_domain::identity::actors::Actor;
use scylla_domain::projects::jobs::{Job, NewJob};
use scylla_domain::projects::pipelines::{
    EnvValue, EnvVar, ListPipelines, NewPipeline, Node, Pipeline, PipelineChange, PipelineId,
    PipelineLayout, PipelinePage, KIND,
};
use scylla_domain::projects::triggers::TriggerId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::{CreateContext, DeleteContext, Hooks, Tx};
use sqlx::PgPool;

use super::store::{self, Changed};
use crate::features::identity::agents::Registry;
use crate::features::projects::jobs::{self, runs};
use crate::features::projects::{secrets, service as projects};
use crate::Error;

/// The largest JSON of the nodes of a pipeline, in bytes.
const MAX_NODES_JSON: usize = 8 << 20;

/// Creates a pipeline in a project if each `BeforeCreate` hook accepts it.
pub(crate) async fn create(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    workspace: WorkspaceId,
    new: NewPipeline,
) -> Result<Pipeline, Error> {
    // Check the size of the nodes, then run the hooks.
    check_size(&new.nodes)?;
    let mut tx = pool.begin().await?;
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

/// Returns a pipeline with its saved places, or `None` if no pipeline has this id.
pub(crate) async fn get_with_layout(
    pool: &PgPool,
    id: PipelineId,
) -> Result<Option<(Pipeline, Option<PipelineLayout>)>, Error> {
    let mut tx = pool.begin().await?;
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

/// Returns one page of pipelines, oldest first.
pub(crate) async fn list(pool: &PgPool, query: ListPipelines) -> Result<PipelinePage, Error> {
    let mut tx = pool.begin().await?;
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

    // Change the pipeline and its places.
    let mut tx = pool.begin().await?;
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
        Changed::Done(pipeline) => Ok(pipeline),
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
    // Delete the row and run the hooks in one transaction.
    let mut tx = pool.begin().await?;
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

/// Starts a pending job of a pipeline with a snapshot of its nodes. A secret that a node names
/// and that the project does not have gives `NOT_FOUND`.
pub(crate) async fn run(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    actor: Actor,
    id: PipelineId,
) -> Result<Job, Error> {
    let pipeline = get(pool, id).await?.ok_or(Error::NotFound("pipeline"))?;
    start(pool, hooks, registry, actor, pipeline, &[], None).await
}

/// Starts a pending job of a pipeline, with more variables in each node and the trigger of the
/// job. A variable of a node with the same name wins. A secret that a node names and that the
/// project does not have gives `NOT_FOUND`.
pub(crate) async fn start(
    pool: &PgPool,
    hooks: &Hooks,
    registry: &Registry,
    actor: Actor,
    pipeline: Pipeline,
    extra: &[EnvVar],
    trigger_id: Option<TriggerId>,
) -> Result<Job, Error> {
    // Refuse a missing secret.
    let names: Vec<String> = pipeline
        .nodes
        .iter()
        .flat_map(|node| &node.env)
        .filter_map(|var| match &var.value {
            EnvValue::Secret(name) => Some(name.as_str().to_owned()),
            EnvValue::Literal(_) => None,
        })
        .collect();
    if let Some(name) = secrets::missing(pool, pipeline.project_id, &names).await? {
        tracing::info!(pipeline = %pipeline.id, secret = name, "a pipeline names a missing secret");
        return Err(Error::NotFound("secret"));
    }

    // Add the variables that a node does not have, then create the job.
    let mut nodes = pipeline.nodes;
    for node in &mut nodes {
        for var in extra {
            if !node.env.iter().any(|own| own.name == var.name) {
                node.env.push(var.clone());
            }
        }
    }
    let new = NewJob {
        pipeline_id: pipeline.id,
        project_id: pipeline.project_id,
        workspace_id: pipeline.workspace_id,
        created_by: actor.id,
        rerun_of: None,
        trigger_id,
        nodes,
    };
    jobs::create(pool, hooks, registry, actor, new).await
}

/// Refuses nodes whose JSON is larger than 8 MiB.
fn check_size(nodes: &scylla_domain::projects::pipelines::Nodes) -> Result<(), Error> {
    let size = super::json::nodes(nodes.as_slice()).to_string().len();
    if size > MAX_NODES_JSON {
        return Err(Error::invalid("nodes", "must be at most 8 MiB in JSON"));
    }
    Ok(())
}
