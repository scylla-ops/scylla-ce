//! A row of the `triggers` table and the JSON of the rules of a Git trigger.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{FireResult, Trigger, TriggerId, TriggerKind};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use serde::de::DeserializeOwned;
use serde_json::Value;
use time::OffsetDateTime;

use crate::features::projects::triggers::inputs;
use crate::Error;

/// A row of the `triggers` table, with no secret.
pub(super) struct Row {
    /// The id.
    pub(super) id: uuid::Uuid,
    /// The pipeline.
    pub(super) pipeline_id: uuid::Uuid,
    /// The project.
    pub(super) project_id: uuid::Uuid,
    /// The workspace.
    pub(super) workspace_id: uuid::Uuid,
    /// The author.
    pub(super) created_by: uuid::Uuid,
    /// The display name.
    pub(super) name: String,
    /// The kind.
    pub(super) kind: String,
    /// The schedule of a cron trigger.
    pub(super) schedule: Option<String>,
    /// The rule of a Git trigger in JSON.
    pub(super) rules: Option<Value>,
    /// The inputs in JSON.
    pub(super) inputs: Value,
    /// `true` if the trigger fires.
    pub(super) enabled: bool,
    /// The next fire.
    pub(super) next_fire_at: Option<OffsetDateTime>,
    /// The last fire.
    pub(super) last_fired_at: Option<OffsetDateTime>,
    /// The result of the last fire.
    pub(super) last_status: Option<String>,
    /// The reason of a failed fire.
    pub(super) last_error: Option<String>,
    /// The creation time.
    pub(super) created_at: OffsetDateTime,
    /// The time of the last change.
    pub(super) updated_at: OffsetDateTime,
}

impl Row {
    /// Makes the trigger of the row.
    pub(super) fn into_trigger(self) -> Result<Trigger, Error> {
        let bad = |what: &str| Error::Internal(format!("bad trigger data: {what}"));
        let kind = kind_of(&self.kind, self.schedule, self.rules).ok_or_else(|| bad("kind"))?;
        let inputs = inputs::read(self.inputs).ok_or_else(|| bad("inputs"))?;
        let last_result = match self.last_status.as_deref() {
            Some("ok") => Some(FireResult::Ok),
            Some(_) => Some(FireResult::Error(self.last_error.unwrap_or_default())),
            None => None,
        };
        Ok(Trigger {
            id: TriggerId::from_db(self.id),
            pipeline_id: PipelineId::from_db(self.pipeline_id),
            project_id: ProjectId::from_db(self.project_id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            created_by: ActorId::from_db(self.created_by),
            name: self.name,
            kind,
            inputs,
            enabled: self.enabled,
            next_fire_at: self.next_fire_at,
            last_fired_at: self.last_fired_at,
            last_result,
            created_at: self.created_at,
            updated_at: self.updated_at,
            from_file: false,
        })
    }
}

/// Returns the kind of the columns `kind`, `schedule` and `rules`, or `None` for bad data.
pub(super) fn kind_of(
    kind: &str,
    schedule: Option<String>,
    rules: Option<Value>,
) -> Option<TriggerKind> {
    match (kind, schedule, rules) {
        ("cron", Some(schedule), None) => Some(TriggerKind::Cron(schedule)),
        ("webhook", None, None) => Some(TriggerKind::Webhook),
        ("push", None, Some(rules)) => rule(rules).map(TriggerKind::Push),
        ("pull_request", None, Some(rules)) => rule(rules).map(TriggerKind::PullRequest),
        ("tag", None, Some(rules)) => rule(rules).map(TriggerKind::Tag),
        _ => None,
    }
}

/// Reads the rule of a Git trigger from its JSON, or `None` for bad data.
fn rule<T: DeserializeOwned>(rules: Value) -> Option<T> {
    serde_json::from_value(rules).ok()
}

/// Returns the JSON of the rule of a Git trigger, or `None` for a cron or webhook trigger.
pub(super) fn rules_json(kind: &TriggerKind) -> Option<Value> {
    let rules = match kind {
        TriggerKind::Push(rule) => serde_json::to_value(rule),
        TriggerKind::PullRequest(rule) => serde_json::to_value(rule),
        TriggerKind::Tag(rule) => serde_json::to_value(rule),
        TriggerKind::Cron(_) | TriggerKind::Webhook => return None,
    };
    rules.ok()
}
