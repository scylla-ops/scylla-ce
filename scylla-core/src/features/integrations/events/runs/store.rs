//! The SQL of the runs of the Git events: the enabled Git triggers of the active editor pipelines
//! of a project, and the record of a fire on a trigger.

use scylla_domain::projects::git_rules::GitRules;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{FireResult, Input, TriggerId};
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;
use serde_json::Value;

use crate::features::projects::triggers::inputs;
use crate::Error;

/// An enabled Git trigger of an active editor pipeline.
pub(super) struct GitTrigger {
    /// The trigger.
    pub(super) id: TriggerId,
    /// The pipeline.
    pub(super) pipeline_id: PipelineId,
    /// The rule of the trigger, alone in its rules.
    pub(super) rules: GitRules,
    /// The inputs.
    pub(super) inputs: Vec<Input>,
}

/// Returns the enabled push, pull request and tag triggers of the editor pipelines of a project
/// that are not archived, oldest first.
pub(super) async fn git_triggers(
    tx: &mut Tx<'_>,
    project: ProjectId,
) -> Result<Vec<GitTrigger>, Error> {
    let rows = sqlx::query!(
        r#"SELECT t.id, t.pipeline_id, t.kind, t.rules AS "rules!", t.inputs
           FROM triggers t JOIN pipelines p ON p.id = t.pipeline_id
           WHERE t.project_id = $1 AND t.enabled AND t.kind IN ('push', 'pull_request', 'tag')
             AND p.source = 'editor' AND p.archived_at IS NULL
           ORDER BY t.id"#,
        project.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            let bad = |what: &str| Error::Internal(format!("bad trigger data: {what}"));
            Ok(GitTrigger {
                id: TriggerId::from_db(row.id),
                pipeline_id: PipelineId::from_db(row.pipeline_id),
                rules: rules_of(&row.kind, row.rules).ok_or_else(|| bad("rules"))?,
                inputs: inputs::read(row.inputs).ok_or_else(|| bad("inputs"))?,
            })
        })
        .collect()
}

/// Records the result of a fire on a trigger.
pub(super) async fn record(
    tx: &mut Tx<'_>,
    id: TriggerId,
    result: &FireResult,
) -> Result<(), sqlx::Error> {
    let (status, error) = match result {
        FireResult::Ok => ("ok", None),
        FireResult::Error(reason) => ("error", Some(reason.as_str())),
    };
    sqlx::query!(
        "UPDATE triggers SET last_fired_at = now(), last_status = $2, last_error = $3 WHERE id = $1",
        id.to_db(),
        status,
        error,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the rule of a Git kind from its JSON, alone in its rules, or `None` for bad data.
fn rules_of(kind: &str, rules: Value) -> Option<GitRules> {
    let mut found = GitRules::default();
    match kind {
        "push" => found.push = Some(serde_json::from_value(rules).ok()?),
        "pull_request" => found.pull_request = Some(serde_json::from_value(rules).ok()?),
        "tag" => found.tag = Some(serde_json::from_value(rules).ok()?),
        _ => return None,
    }
    Some(found)
}
