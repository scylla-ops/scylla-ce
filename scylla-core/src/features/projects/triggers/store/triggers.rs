//! The SQL of the `triggers` table.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{FireResult, NewTrigger, Trigger, TriggerId, TriggerKind};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::{Rejection, Tx};
use serde_json::Value;
use time::OffsetDateTime;

use crate::features::projects::triggers::inputs;
use crate::Error;

/// The unique name of a trigger in its pipeline.
const NAME_KEY: &str = "triggers_pipeline_id_name_key";

/// Inserts a trigger of a pipeline, with the place of the pipeline. Returns `None` if no
/// pipeline has this id. A name in use gives `TRIGGER_NAME_IN_USE`.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    id: TriggerId,
    new: &NewTrigger,
    sealed_secret: Option<Vec<u8>>,
    next_fire_at: Option<OffsetDateTime>,
) -> Result<Option<Trigger>, Error> {
    let row = sqlx::query_as!(
        Row,
        r#"INSERT INTO triggers (id, pipeline_id, project_id, workspace_id, created_by, name, kind,
                                schedule, sealed_secret, inputs, next_fire_at)
           SELECT $1, p.id, p.project_id, p.workspace_id, $3, $4, $5, $6, $7, $8, $9
           FROM pipelines p WHERE p.id = $2
           RETURNING id, pipeline_id, project_id, workspace_id, created_by, name, kind, schedule,
                     inputs, enabled, next_fire_at, last_fired_at, last_status, last_error,
                     created_at, updated_at"#,
        id.to_db(),
        new.pipeline_id.to_db(),
        new.created_by.to_db(),
        new.name.as_str(),
        new.kind.name(),
        new.kind.schedule(),
        sealed_secret,
        inputs::json(new.inputs.as_slice()),
        next_fire_at,
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(NAME_KEY) => {
            let message = "another trigger of the pipeline has this name";
            Error::Rejected(Rejection::new("TRIGGER_NAME_IN_USE", message))
        }
        error => error.into(),
    })?;
    row.map(Row::into_trigger).transpose()
}

/// Returns a trigger, or `None` if no trigger has this id.
pub(crate) async fn find(tx: &mut Tx<'_>, id: TriggerId) -> Result<Option<Trigger>, Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, pipeline_id, project_id, workspace_id, created_by, name, kind, schedule, inputs,
                enabled, next_fire_at, last_fired_at, last_status, last_error, created_at,
                updated_at
         FROM triggers WHERE id = $1",
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(Row::into_trigger).transpose()
}

/// Returns the sealed secret of an enabled webhook trigger of an active pipeline, or `None`. A
/// pipeline is not active when it is archived, or when its file left the default branch.
pub(crate) async fn webhook_secret(
    tx: &mut Tx<'_>,
    id: TriggerId,
) -> Result<Option<Vec<u8>>, sqlx::Error> {
    let sealed = sqlx::query_scalar!(
        r#"SELECT t.sealed_secret AS "sealed_secret!"
           FROM triggers t JOIN pipelines p ON p.id = t.pipeline_id
           WHERE t.id = $1 AND t.enabled AND t.kind = 'webhook' AND p.archived_at IS NULL
             AND p.removed_at IS NULL"#,
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(sealed)
}

/// Returns the project and the workspace of a trigger, or `None` if no trigger has this id.
pub(crate) async fn place(
    tx: &mut Tx<'_>,
    id: TriggerId,
) -> Result<Option<(ProjectId, WorkspaceId)>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT project_id, workspace_id FROM triggers WHERE id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| {
        (
            ProjectId::from_db(row.project_id),
            WorkspaceId::from_db(row.workspace_id),
        )
    }))
}

/// Returns the triggers of a pipeline after the page id, in id order, with one extra row.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    pipeline: PipelineId,
    page: PageRequest<TriggerId>,
) -> Result<Vec<Trigger>, Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, pipeline_id, project_id, workspace_id, created_by, name, kind, schedule, inputs,
                enabled, next_fire_at, last_fired_at, last_status, last_error, created_at,
                updated_at
         FROM triggers WHERE pipeline_id = $1 AND ($2::uuid IS NULL OR id > $2)
         ORDER BY id LIMIT $3",
        pipeline.to_db(),
        page.after.map(TriggerId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_trigger).collect()
}

/// Writes the changes of a trigger that the caller read in this transaction. A name in use gives
/// `TRIGGER_NAME_IN_USE`.
pub(crate) async fn save(tx: &mut Tx<'_>, trigger: &Trigger) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE triggers SET name = $2, schedule = $3, inputs = $4, enabled = $5,
                             next_fire_at = $6, updated_at = now()
         WHERE id = $1",
        trigger.id.to_db(),
        trigger.name,
        trigger.kind.schedule(),
        inputs::json(&trigger.inputs),
        trigger.enabled,
        trigger.next_fire_at,
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(NAME_KEY) => {
            let message = "another trigger of the pipeline has this name";
            Error::Rejected(Rejection::new("TRIGGER_NAME_IN_USE", message))
        }
        error => error.into(),
    })?;
    Ok(())
}

/// Deletes a trigger. Returns `false` if no trigger has this id.
pub(crate) async fn delete(tx: &mut Tx<'_>, id: TriggerId) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM triggers WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// Locks at most `limit` enabled cron triggers that are due, with their schedules and `true`
/// when their pipeline is active: not archived, and with its file on the default branch. Another
/// server skips the locked rows.
pub(crate) async fn due(
    tx: &mut Tx<'_>,
    limit: i64,
) -> Result<Vec<(TriggerId, String, bool)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"SELECT t.id, t.schedule AS "schedule!",
                  (SELECT p.archived_at IS NULL AND p.removed_at IS NULL
                   FROM pipelines p WHERE p.id = t.pipeline_id) AS "active!"
           FROM triggers t
           WHERE t.enabled AND t.kind = 'cron' AND t.next_fire_at <= now()
           ORDER BY t.next_fire_at LIMIT $1 FOR UPDATE OF t SKIP LOCKED"#,
        limit,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (TriggerId::from_db(row.id), row.schedule, row.active))
        .collect())
}

/// Sets the next fire of a cron trigger.
pub(crate) async fn set_next(
    tx: &mut Tx<'_>,
    id: TriggerId,
    next: Option<OffsetDateTime>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE triggers SET next_fire_at = $2 WHERE id = $1",
        id.to_db(),
        next
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Records the result of a fire.
pub(crate) async fn record(
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

/// A row of the `triggers` table, with no secret.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The pipeline.
    pipeline_id: uuid::Uuid,
    /// The project.
    project_id: uuid::Uuid,
    /// The workspace.
    workspace_id: uuid::Uuid,
    /// The author.
    created_by: uuid::Uuid,
    /// The display name.
    name: String,
    /// The kind.
    kind: String,
    /// The schedule of a cron trigger.
    schedule: Option<String>,
    /// The inputs in JSON.
    inputs: Value,
    /// `true` if the trigger fires.
    enabled: bool,
    /// The next fire.
    next_fire_at: Option<OffsetDateTime>,
    /// The last fire.
    last_fired_at: Option<OffsetDateTime>,
    /// The result of the last fire.
    last_status: Option<String>,
    /// The reason of a failed fire.
    last_error: Option<String>,
    /// The creation time.
    created_at: OffsetDateTime,
    /// The time of the last change.
    updated_at: OffsetDateTime,
}

impl Row {
    /// Makes the trigger of the row.
    fn into_trigger(self) -> Result<Trigger, Error> {
        let bad = |what: &str| Error::Internal(format!("bad trigger data: {what}"));
        let kind = match (self.kind.as_str(), self.schedule) {
            ("cron", Some(schedule)) => TriggerKind::Cron(schedule),
            ("webhook", None) => TriggerKind::Webhook,
            _ => return Err(bad("kind")),
        };
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
