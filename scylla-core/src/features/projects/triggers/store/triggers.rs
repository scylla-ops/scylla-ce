//! The SQL of the `triggers` table.

use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{FireResult, NewTrigger, Trigger, TriggerId, TriggerKind};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::{Rejection, Tx};
use time::OffsetDateTime;

use super::row::{self, Row};
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
                                schedule, sealed_secret, rules, inputs, next_fire_at)
           SELECT $1, p.id, p.project_id, p.workspace_id, $3, $4, $5, $6, $7, $8, $9, $10
           FROM pipelines p WHERE p.id = $2
           RETURNING id, pipeline_id, project_id, workspace_id, created_by, name, kind, schedule,
                     rules, inputs, enabled, next_fire_at, last_fired_at, last_status, last_error,
                     created_at, updated_at"#,
        id.to_db(),
        new.pipeline_id.to_db(),
        new.created_by.to_db(),
        new.name.as_str(),
        new.kind.name(),
        new.kind.schedule(),
        sealed_secret,
        row::rules_json(&new.kind),
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
        "SELECT id, pipeline_id, project_id, workspace_id, created_by, name, kind, schedule, rules,
                inputs, enabled, next_fire_at, last_fired_at, last_status, last_error, created_at,
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

/// Returns two flags of a pipeline: it comes from a file, and its project has a linked
/// repository. `None` if no pipeline has this id.
pub(crate) async fn git_place(
    tx: &mut Tx<'_>,
    pipeline: PipelineId,
) -> Result<Option<(bool, bool)>, sqlx::Error> {
    let row = sqlx::query!(
        r#"SELECT p.source = 'file' AS "file!",
                  EXISTS (SELECT 1 FROM repository_links l WHERE l.project_id = p.project_id)
                      AS "linked!"
           FROM pipelines p WHERE p.id = $1"#,
        pipeline.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| (row.file, row.linked)))
}

/// Returns the kinds of the enabled Git triggers of a pipeline, oldest first.
pub(crate) async fn git_kinds(
    tx: &mut Tx<'_>,
    pipeline: PipelineId,
) -> Result<Vec<TriggerKind>, Error> {
    let rows = sqlx::query!(
        "SELECT kind, rules FROM triggers
         WHERE pipeline_id = $1 AND enabled AND kind IN ('push', 'pull_request', 'tag')
         ORDER BY id",
        pipeline.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            row::kind_of(&row.kind, None, row.rules)
                .ok_or_else(|| Error::Internal("bad trigger data: rules".to_owned()))
        })
        .collect()
}

/// Returns the triggers of a pipeline after the page id, in id order, with one extra row.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    pipeline: PipelineId,
    page: PageRequest<TriggerId>,
) -> Result<Vec<Trigger>, Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, pipeline_id, project_id, workspace_id, created_by, name, kind, schedule, rules,
                inputs, enabled, next_fire_at, last_fired_at, last_status, last_error, created_at,
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
        "UPDATE triggers SET name = $2, schedule = $3, rules = $4, inputs = $5, enabled = $6,
                             next_fire_at = $7, updated_at = now()
         WHERE id = $1",
        trigger.id.to_db(),
        trigger.name,
        trigger.kind.schedule(),
        row::rules_json(&trigger.kind),
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
