//! The opt-in of a workspace admin for the Git events of an editor pipeline: in a project of
//! files, only then do its push, pull request and tag triggers start runs.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::{Pipeline, PipelineId};
use sqlx::PgPool;

use super::{service, store};
use crate::Error;

/// Lets the Git triggers of an editor pipeline start runs, or stops them. The triggers stay. A
/// file pipeline or an archived pipeline gives `PIPELINE_READ_ONLY`. The version does not change.
pub(super) async fn set(
    pool: &PgPool,
    id: PipelineId,
    actor: ActorId,
    allowed: bool,
) -> Result<Pipeline, Error> {
    // Refuse a read only pipeline, then change the opt-in.
    let mut tx = pool.begin().await?;
    service::refuse_read_only(&mut tx, id).await?;
    store::set_git_events(&mut tx, id, allowed.then_some(actor)).await?;
    let pipeline = store::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    tx.commit().await?;
    tracing::info!(event = "pipeline.git_events_changed", pipeline = %id, allowed);
    Ok(pipeline)
}
