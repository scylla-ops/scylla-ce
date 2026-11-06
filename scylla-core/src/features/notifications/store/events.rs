//! The SQL of the facts: the outbox that the dispatcher reads.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Event, EventRow, Params, Tx};
use sqlx::types::Json;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// The name of a project and of a workspace, for the snapshot of a fact.
pub(crate) struct Names {
    /// The name of the workspace.
    pub(crate) workspace: Option<String>,
    /// The name of the project.
    pub(crate) project: Option<String>,
}

/// Returns the names of a workspace and of a project.
pub(crate) async fn names(
    tx: &mut Tx<'_>,
    workspace: Option<WorkspaceId>,
    project: Option<ProjectId>,
) -> Result<Names, Error> {
    let row = sqlx::query!(
        r#"SELECT (SELECT name FROM workspaces WHERE id = $1) AS workspace,
                  (SELECT name FROM projects WHERE id = $2) AS project"#,
        workspace.map(WorkspaceId::to_db),
        project.map(ProjectId::to_db),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Names {
        workspace: row.workspace,
        project: row.project,
    })
}

/// Records a fact. Returns `false` if its dedupe key has a record.
pub(crate) async fn insert(tx: &mut Tx<'_>, event: &Event) -> Result<bool, Error> {
    let (subject_kind, subject_id) = event.subject.clone().unzip();
    let inserted = sqlx::query!(
        "INSERT INTO notification_events (id, kind, actor_id, workspace_id, project_id,
             subject_kind, subject_id, params, dedupe_key, group_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (dedupe_key) WHERE dedupe_key IS NOT NULL DO NOTHING",
        Uuid::now_v7(),
        event.kind,
        event.actor.map(ActorId::to_db),
        event.workspace.map(WorkspaceId::to_db),
        event.project.map(ProjectId::to_db),
        subject_kind,
        subject_id,
        Json(&event.params) as _,
        event.dedupe,
        event.group,
    )
    .execute(&mut **tx)
    .await?;
    Ok(inserted.rows_affected() == 1)
}

/// Returns up to `limit` facts that wait, oldest first, with the attempts of each.
pub(crate) async fn pending(tx: &mut Tx<'_>, limit: i64) -> Result<Vec<(EventRow, i32)>, Error> {
    let rows = sqlx::query!(
        r#"SELECT id, kind, actor_id, workspace_id, project_id, subject_kind, subject_id,
                  params AS "params: Json<Params>", group_key, attempts
           FROM notification_events
           WHERE dispatched_at IS NULL AND next_attempt_at <= now()
           ORDER BY created_at, id
           LIMIT $1"#,
        limit,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let mut event = Event::new(row.kind);
            event.actor = row.actor_id.map(ActorId::from_db);
            event.workspace = row.workspace_id.map(WorkspaceId::from_db);
            event.project = row.project_id.map(ProjectId::from_db);
            event.subject = row.subject_kind.zip(row.subject_id);
            event.params = row.params.0;
            event.group = row.group_key;
            (EventRow::new(row.id, event), row.attempts)
        })
        .collect())
}

/// Marks a fact as dispatched. Returns `false` if another pass did it first.
pub(crate) async fn claim(tx: &mut Tx<'_>, id: Uuid) -> Result<bool, Error> {
    let claimed = sqlx::query!(
        "UPDATE notification_events SET dispatched_at = now()
         WHERE id = $1 AND dispatched_at IS NULL",
        id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(claimed.rows_affected() == 1)
}

/// Adds one attempt to a fact, and sets the time of the next one.
pub(crate) async fn retry_at(tx: &mut Tx<'_>, id: Uuid, at: OffsetDateTime) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notification_events SET attempts = attempts + 1, next_attempt_at = $2
         WHERE id = $1 AND dispatched_at IS NULL",
        id,
        at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the facts of an object that wait.
pub(crate) async fn delete_pending(
    tx: &mut Tx<'_>,
    subject_kind: &str,
    subject: Uuid,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM notification_events
         WHERE subject_kind = $1 AND subject_id = $2 AND dispatched_at IS NULL",
        subject_kind,
        subject,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the facts that were dispatched before a time. Returns their number.
pub(crate) async fn purge(tx: &mut Tx<'_>, before: OffsetDateTime) -> Result<u64, Error> {
    let deleted = sqlx::query!(
        "DELETE FROM notification_events WHERE dispatched_at < $1",
        before,
    )
    .execute(&mut **tx)
    .await?;
    Ok(deleted.rows_affected())
}
