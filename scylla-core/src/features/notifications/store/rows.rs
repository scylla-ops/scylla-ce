//! The SQL of the rows of the inbox that a fact writes.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Event, Params, Recipient, Tx};
use sqlx::types::Json;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// The open row of a group.
pub(crate) struct OpenRow {
    /// The id of the row.
    pub(crate) id: Uuid,
    /// The parameters of the row.
    pub(crate) params: Params,
    /// The time of the last fact.
    pub(crate) last_at: OffsetDateTime,
}

/// Returns the actors that are active users.
pub(crate) async fn active_users(
    tx: &mut Tx<'_>,
    actors: &[ActorId],
) -> Result<Vec<ActorId>, Error> {
    let ids: Vec<Uuid> = actors.iter().map(|actor| actor.to_db()).collect();
    let rows = sqlx::query_scalar!(
        "SELECT id FROM users WHERE id = ANY($1) AND status = 'active'",
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(ActorId::from_db).collect())
}

/// Inserts a row of a fact, and returns `false` if its group has an open row.
pub(crate) async fn insert_new(
    tx: &mut Tx<'_>,
    recipient: Recipient,
    event: &Event,
    group: &str,
    needs_action: bool,
) -> Result<bool, Error> {
    let (subject_kind, subject_id) = event.subject.clone().unzip();
    let inserted = sqlx::query!(
        "INSERT INTO notifications (id, recipient_id, kind, reason, actor_id, workspace_id,
             project_id, subject_kind, subject_id, params, group_key, needs_action)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (recipient_id, group_key) WHERE closed_at IS NULL DO NOTHING",
        Uuid::now_v7(),
        recipient.user.to_db(),
        event.kind,
        recipient.reason,
        event.actor.map(ActorId::to_db),
        event.workspace.map(WorkspaceId::to_db),
        event.project.map(ProjectId::to_db),
        subject_kind,
        subject_id,
        Json(&event.params) as _,
        group,
        needs_action,
    )
    .execute(&mut **tx)
    .await?;
    Ok(inserted.rows_affected() == 1)
}

/// Adds 1 to the count of the open row of a group, moves it up, and makes it unread.
pub(crate) async fn count(
    tx: &mut Tx<'_>,
    recipient: ActorId,
    event: &Event,
    group: &str,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notifications SET occurrences = occurrences + 1, last_at = now(), read_at = NULL,
             done_at = NULL, params = $3, actor_id = $4
         WHERE recipient_id = $1 AND group_key = $2 AND closed_at IS NULL",
        recipient.to_db(),
        group,
        Json(&event.params) as _,
        event.actor.map(ActorId::to_db),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Replaces the parameters and the actor of the open row of a group.
pub(crate) async fn replace(
    tx: &mut Tx<'_>,
    recipient: ActorId,
    event: &Event,
    group: &str,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notifications SET params = $3, actor_id = $4
         WHERE recipient_id = $1 AND group_key = $2 AND closed_at IS NULL",
        recipient.to_db(),
        group,
        Json(&event.params) as _,
        event.actor.map(ActorId::to_db),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Locks and returns the open row of a group.
pub(crate) async fn lock_open(
    tx: &mut Tx<'_>,
    recipient: ActorId,
    group: &str,
) -> Result<Option<OpenRow>, Error> {
    let row = sqlx::query!(
        r#"SELECT id, params AS "params: Json<Params>", last_at FROM notifications
           WHERE recipient_id = $1 AND group_key = $2 AND closed_at IS NULL
           FOR UPDATE"#,
        recipient.to_db(),
        group,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| OpenRow {
        id: row.id,
        params: row.params.0,
        last_at: row.last_at,
    }))
}

/// Writes the merged parameters of a row, with the kind and the actor of the last fact.
pub(crate) async fn merged(
    tx: &mut Tx<'_>,
    id: Uuid,
    params: &Params,
    event: &Event,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notifications SET params = $2, kind = $3, actor_id = $4,
             occurrences = occurrences + 1, last_at = now(), read_at = NULL, done_at = NULL
         WHERE id = $1",
        id,
        Json(params) as _,
        event.kind,
        event.actor.map(ActorId::to_db),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Closes one row, so the next fact of its group makes a new row.
pub(crate) async fn close_row(tx: &mut Tx<'_>, id: Uuid) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notifications SET closed_at = now(), needs_action = false WHERE id = $1",
        id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Closes the rows of a kind on an object, and marks them as read: the reader answered.
pub(crate) async fn close_subject(
    tx: &mut Tx<'_>,
    kind: &str,
    subject_kind: &str,
    subject: Uuid,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notifications
         SET closed_at = now(), needs_action = false, read_at = COALESCE(read_at, now())
         WHERE kind = $1 AND subject_kind = $2 AND subject_id = $3 AND closed_at IS NULL",
        kind,
        subject_kind,
        subject,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Opens the closed rows of a kind on an object again, except where the group has an open row.
pub(crate) async fn reopen_subject(
    tx: &mut Tx<'_>,
    kind: &str,
    subject_kind: &str,
    subject: Uuid,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notifications AS row
         SET closed_at = NULL, needs_action = true, read_at = NULL, done_at = NULL,
             last_at = now()
         WHERE row.kind = $1 AND row.subject_kind = $2 AND row.subject_id = $3
           AND (row.closed_at IS NOT NULL OR row.done_at IS NOT NULL)
           AND NOT EXISTS (
               SELECT 1 FROM notifications AS open
               WHERE open.recipient_id = row.recipient_id AND open.group_key = row.group_key
                 AND open.closed_at IS NULL AND open.id <> row.id)",
        kind,
        subject_kind,
        subject,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the rows of an object.
pub(crate) async fn delete_subject(
    tx: &mut Tx<'_>,
    subject_kind: &str,
    subject: Uuid,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM notifications WHERE subject_kind = $1 AND subject_id = $2",
        subject_kind,
        subject,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the owners of a workspace.
pub(crate) async fn owners(tx: &mut Tx<'_>, workspace: WorkspaceId) -> Result<Vec<ActorId>, Error> {
    let rows = sqlx::query_scalar!(
        "SELECT user_id FROM workspace_owners WHERE workspace_id = $1",
        workspace.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(ActorId::from_db).collect())
}

/// Returns the instance admins.
pub(crate) async fn admins(tx: &mut Tx<'_>) -> Result<Vec<ActorId>, Error> {
    let rows = sqlx::query_scalar!("SELECT user_id FROM instance_admins")
        .fetch_all(&mut **tx)
        .await?;
    Ok(rows.into_iter().map(ActorId::from_db).collect())
}
