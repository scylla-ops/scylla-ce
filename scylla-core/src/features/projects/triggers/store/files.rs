//! The SQL of the triggers of a file pipeline: the rules of the `on` table of its file, as the
//! sync keeps them in `pipelines.file_rules`. They have no row in `triggers`.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::git_rules::GitRules;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{Trigger, TriggerId, TriggerKind};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use crate::Error;

/// Returns the triggers of the `on` table of a file pipeline: push, pull request, then tag. Each
/// one has an id that comes from the pipeline and its kind, and it is enabled while the pipeline
/// is active. An editor pipeline has none.
pub(crate) async fn list(tx: &mut Tx<'_>, pipeline: PipelineId) -> Result<Vec<Trigger>, Error> {
    // Read the rules of the file.
    let row = sqlx::query!(
        r#"SELECT project_id, workspace_id, created_by, file_rules AS "rules!", created_at,
                  updated_at, archived_at IS NULL AND removed_at IS NULL AS "active!",
                  md5(id::text || ':push')::uuid AS "push!",
                  md5(id::text || ':pull_request')::uuid AS "pull_request!",
                  md5(id::text || ':tag')::uuid AS "tag!"
           FROM pipelines
           WHERE id = $1 AND source = 'file' AND file_rules IS NOT NULL"#,
        pipeline.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(row) = row else {
        return Ok(Vec::new());
    };
    let rules: GitRules = serde_json::from_value(row.rules)
        .map_err(|error| Error::Internal(format!("bad rules of a pipeline file: {error}")))?;

    // Make one trigger for each rule.
    let kinds = [
        (row.push, "on.push", rules.push.map(TriggerKind::Push)),
        (
            row.pull_request,
            "on.pull_request",
            rules.pull_request.map(TriggerKind::PullRequest),
        ),
        (row.tag, "on.tag", rules.tag.map(TriggerKind::Tag)),
    ];
    let triggers = kinds
        .into_iter()
        .filter_map(|(id, name, kind)| {
            kind.map(|kind| Trigger {
                id: TriggerId::from_db(id),
                pipeline_id: pipeline,
                project_id: ProjectId::from_db(row.project_id),
                workspace_id: WorkspaceId::from_db(row.workspace_id),
                created_by: ActorId::from_db(row.created_by),
                name: name.to_owned(),
                kind,
                inputs: Vec::new(),
                enabled: row.active,
                next_fire_at: None,
                last_fired_at: None,
                last_result: None,
                created_at: row.created_at,
                updated_at: row.updated_at,
                from_file: true,
            })
        })
        .collect();
    Ok(triggers)
}
