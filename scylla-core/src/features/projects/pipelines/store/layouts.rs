//! The SQL of the `pipeline_layouts` table: the places of the nodes of a pipeline.

use scylla_domain::projects::pipelines::{PipelineId, PipelineLayout};
use scylla_extension::Tx;

use super::super::json;
use crate::Error;

/// Returns the saved places of a pipeline, or `None` if nothing is saved.
pub(crate) async fn find_layout(
    tx: &mut Tx<'_>,
    id: PipelineId,
) -> Result<Option<PipelineLayout>, Error> {
    let row = sqlx::query!(
        "SELECT start, nodes FROM pipeline_layouts WHERE pipeline_id = $1",
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    let layout = row
        .map(|row| json::read_layout(row.start, row.nodes))
        .transpose()?;
    Ok(layout.filter(|layout| !layout.is_empty()))
}

/// Locks a pipeline, so a change of its nodes waits, and returns the ids of its nodes. Returns
/// `None` if no pipeline has this id.
pub(crate) async fn lock_node_ids(
    tx: &mut Tx<'_>,
    id: PipelineId,
) -> Result<Option<Vec<String>>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"SELECT ARRAY(SELECT node ->> 'id' FROM jsonb_array_elements(nodes) AS node)
               AS "ids!: Vec<String>"
           FROM pipelines WHERE id = $1 FOR SHARE"#,
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await
}

/// Saves places of a pipeline in one statement, so two saves at one time both stay. The other
/// places stay.
pub(crate) async fn save_layout(
    tx: &mut Tx<'_>,
    id: PipelineId,
    layout: &PipelineLayout,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO pipeline_layouts (pipeline_id, start, nodes) VALUES ($1, $2, $3)
         ON CONFLICT (pipeline_id) DO UPDATE
         SET start = coalesce(excluded.start, pipeline_layouts.start),
             nodes = pipeline_layouts.nodes || excluded.nodes,
             updated_at = now()",
        id.to_db(),
        layout.start.map(json::place),
        json::places(&layout.nodes),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Removes the places of the ids that are not in a list.
pub(crate) async fn keep_places(
    tx: &mut Tx<'_>,
    id: PipelineId,
    nodes: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE pipeline_layouts
         SET nodes = (SELECT coalesce(jsonb_object_agg(key, value), '{}')
                      FROM jsonb_each(nodes) WHERE key = ANY($2)),
             updated_at = now()
         WHERE pipeline_id = $1",
        id.to_db(),
        nodes,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
