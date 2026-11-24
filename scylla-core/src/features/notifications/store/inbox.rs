//! The SQL of the inbox that a reader sees and changes.

use std::collections::BTreeMap;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::notifications::{
    Filter, InvitationState, Notification, NotificationId, RowInvitation, Summary, UNREAD_CAP,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::invitations::InvitationId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;
use sqlx::types::Json;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// Returns the counts of an inbox. The count of the unread rows stops at `UNREAD_CAP`.
pub(crate) async fn summary(tx: &mut Tx<'_>, user: ActorId) -> Result<Summary, Error> {
    let row = sqlx::query!(
        r#"SELECT
             (SELECT count(*) FROM (
                 SELECT 1 FROM notifications
                 WHERE recipient_id = $1 AND read_at IS NULL AND done_at IS NULL
                 LIMIT $2) AS capped) AS "unread!",
             (SELECT count(*) FROM notifications
              WHERE recipient_id = $1 AND needs_action AND closed_at IS NULL AND done_at IS NULL)
              AS "needs_action!""#,
        user.to_db(),
        i64::from(UNREAD_CAP),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(Summary {
        unread: i32::try_from(row.unread).unwrap_or(UNREAD_CAP),
        needs_action: i32::try_from(row.needs_action).unwrap_or(i32::MAX),
    })
}

/// Returns one page of the rows of a user with one extra row: the rows that need an action first,
/// then the newest.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    user: ActorId,
    filter: &Filter,
    page: PageRequest<NotificationId>,
) -> Result<Vec<Notification>, Error> {
    let rows = sqlx::query!(
        r#"SELECT n.id, n.kind, n.reason, n.workspace_id, n.project_id,
                  n.params AS "params: Json<BTreeMap<String, String>>", n.occurrences,
                  n.needs_action, n.first_at, n.last_at, n.read_at, n.done_at,
                  COALESCE(u.display_name, a.name) AS actor_name,
                  w.slug AS "workspace_slug?", w.name AS "workspace_name?",
                  p.name AS "project_name?",
                  i.id AS "invitation_id?", i.role AS "invitation_role?",
                  i.accepted_at AS "invitation_accepted_at?",
                  i.expires_at AS "invitation_expires_at?"
           FROM notifications AS n
           LEFT JOIN users AS u ON u.id = n.actor_id
           LEFT JOIN apps AS a ON a.id = n.actor_id
           LEFT JOIN workspaces AS w ON w.id = n.workspace_id
           LEFT JOIN projects AS p ON p.id = n.project_id
           LEFT JOIN invitations AS i ON n.subject_kind = 'invitation' AND i.id = n.subject_id
           WHERE n.recipient_id = $1
             AND ((n.done_at IS NOT NULL) = $2)
             AND (NOT $3 OR n.read_at IS NULL)
             AND (NOT $4 OR (n.needs_action AND n.closed_at IS NULL))
             AND ($5::uuid IS NULL OR n.workspace_id = $5)
             AND ($6::text[] IS NULL OR n.kind = ANY($6))
             AND ($7::uuid IS NULL OR (n.needs_action, n.last_at, n.id) < (
                 SELECT c.needs_action, c.last_at, c.id FROM notifications AS c
                 WHERE c.id = $7 AND c.recipient_id = $1))
           ORDER BY n.needs_action DESC, n.last_at DESC, n.id DESC
           LIMIT $8"#,
        user.to_db(),
        filter.done,
        filter.unread_only,
        filter.needs_action_only,
        filter.workspace.map(WorkspaceId::to_db),
        filter.kinds.as_deref(),
        page.after.map(NotificationId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let invitation = match (row.invitation_id, row.invitation_role) {
                (Some(id), Some(role)) => Some(RowInvitation {
                    id: InvitationId::from_db(id),
                    role,
                    state: state(row.invitation_accepted_at, row.invitation_expires_at),
                }),
                _ => None,
            };
            Notification {
                id: NotificationId::from_db(row.id),
                kind: row.kind,
                reason: row.reason,
                actor_name: row.actor_name,
                workspace_id: row.workspace_id.map(WorkspaceId::from_db),
                workspace_slug: row.workspace_slug,
                workspace_name: row.workspace_name,
                project_id: row.project_id.map(ProjectId::from_db),
                project_name: row.project_name,
                params: row.params.0,
                occurrences: row.occurrences,
                needs_action: row.needs_action,
                first_at: row.first_at,
                last_at: row.last_at,
                read_at: row.read_at,
                done_at: row.done_at,
                invitation,
            }
        })
        .collect())
}

/// Returns the state of an invitation at the time of a read.
fn state(
    accepted_at: Option<OffsetDateTime>,
    expires_at: Option<OffsetDateTime>,
) -> InvitationState {
    let expires_at = expires_at.unwrap_or(OffsetDateTime::UNIX_EPOCH);
    match accepted_at {
        Some(at) => InvitationState::Accepted(at),
        None if expires_at > OffsetDateTime::now_utc() => InvitationState::Open(expires_at),
        None => InvitationState::Expired(expires_at),
    }
}

/// Returns the number of rows of a user among some ids.
pub(crate) async fn count_owned(
    tx: &mut Tx<'_>,
    user: ActorId,
    ids: &[Uuid],
) -> Result<i64, Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM notifications
           WHERE recipient_id = $1 AND id = ANY($2)"#,
        user.to_db(),
        ids,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(count)
}

/// Marks rows of a user as read or as unread. A row that waits for an action stays unread.
pub(crate) async fn set_read(
    tx: &mut Tx<'_>,
    user: ActorId,
    ids: &[Uuid],
    read: bool,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notifications
         SET read_at = CASE WHEN $3 THEN COALESCE(read_at, now()) ELSE NULL END
         WHERE recipient_id = $1 AND id = ANY($2)
           AND NOT (needs_action AND closed_at IS NULL)",
        user.to_db(),
        ids,
        read,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Marks each row of a user as read, except a row that waits for an action.
pub(crate) async fn set_all_read(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notifications SET read_at = now()
         WHERE recipient_id = $1 AND read_at IS NULL AND NOT (needs_action AND closed_at IS NULL)",
        user.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Marks rows of a user as done, or puts them back in the inbox.
pub(crate) async fn set_done(
    tx: &mut Tx<'_>,
    user: ActorId,
    ids: &[Uuid],
    done: bool,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE notifications
         SET done_at = CASE WHEN $3 THEN COALESCE(done_at, now()) ELSE NULL END
         WHERE recipient_id = $1 AND id = ANY($2)",
        user.to_db(),
        ids,
        done,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes rows of a user.
pub(crate) async fn delete(tx: &mut Tx<'_>, user: ActorId, ids: &[Uuid]) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM notifications WHERE recipient_id = $1 AND id = ANY($2)",
        user.to_db(),
        ids,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Closes the rows of the invitations that ended, so they no longer wait for an action.
pub(crate) async fn close_ended_invitations(tx: &mut Tx<'_>) -> Result<u64, Error> {
    let closed = sqlx::query!(
        "UPDATE notifications AS n SET closed_at = now(), needs_action = false
         FROM invitations AS i
         WHERE n.subject_kind = 'invitation' AND i.id = n.subject_id AND n.needs_action
           AND n.closed_at IS NULL AND i.expires_at <= now()",
    )
    .execute(&mut **tx)
    .await?;
    Ok(closed.rows_affected())
}

/// Deletes the rows with no fact after a time.
pub(crate) async fn purge_older(tx: &mut Tx<'_>, before: OffsetDateTime) -> Result<u64, Error> {
    let deleted = sqlx::query!("DELETE FROM notifications WHERE last_at < $1", before)
        .execute(&mut **tx)
        .await?;
    Ok(deleted.rows_affected())
}

/// Deletes the rows above the limit of each person: the oldest read rows first.
pub(crate) async fn purge_above(tx: &mut Tx<'_>, limit: i64) -> Result<u64, Error> {
    let deleted = sqlx::query!(
        "DELETE FROM notifications WHERE id IN (
             SELECT id FROM (
                 SELECT id, row_number() OVER (
                     PARTITION BY recipient_id
                     ORDER BY (read_at IS NULL) DESC, last_at DESC, id DESC) AS place
                 FROM notifications
                 WHERE recipient_id IN (
                     SELECT recipient_id FROM notifications
                     GROUP BY recipient_id HAVING count(*) > $1)
             ) AS ranked WHERE place > $1)",
        limit,
    )
    .execute(&mut **tx)
    .await?;
    Ok(deleted.rows_affected())
}
