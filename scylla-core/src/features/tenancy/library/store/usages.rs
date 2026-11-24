//! The SQL of the usages of the fragments: the links in `pipelines.fragment_links`.

use std::collections::{BTreeMap, HashMap};

use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::library::{FragmentId, Usage};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use crate::Error;

/// Returns the active pipelines of a workspace that link each fragment, in pipeline order. A link
/// names its fragment as the text of its id.
pub(crate) async fn of(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    ids: &[FragmentId],
) -> Result<HashMap<FragmentId, Vec<Usage>>, Error> {
    let texts: Vec<String> = ids.iter().map(ToString::to_string).collect();
    let rows = sqlx::query!(
        r#"SELECT l.link->>'fragment_id' AS "fragment_id!", p.id AS pipeline_id,
                  p.name AS pipeline_name, r.id AS project_id, r.name AS project_name,
                  (l.link->>'version')::bigint AS "version!", l.link->'values' AS "values!"
           FROM pipelines p
           JOIN projects r ON r.id = p.project_id
           CROSS JOIN LATERAL jsonb_array_elements(p.fragment_links) AS l(link)
           WHERE p.workspace_id = $1 AND p.archived_at IS NULL
             AND l.link->>'fragment_id' = ANY($2)
           ORDER BY p.id"#,
        workspace.to_db(),
        &texts,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut usages: HashMap<FragmentId, Vec<Usage>> = HashMap::new();
    for row in rows {
        let bad = |error: String| Error::Internal(format!("bad fragment link: {error}"));
        let fragment = FragmentId::parse(&row.fragment_id).map_err(|e| bad(e.to_string()))?;
        let values: BTreeMap<String, String> =
            serde_json::from_value(row.values).map_err(|e| bad(e.to_string()))?;
        usages.entry(fragment).or_default().push(Usage {
            pipeline_id: PipelineId::from_db(row.pipeline_id),
            pipeline_name: row.pipeline_name,
            project_id: ProjectId::from_db(row.project_id),
            project_name: row.project_name,
            version: u32::try_from(row.version).unwrap_or_default(),
            values,
        });
    }
    Ok(usages)
}
