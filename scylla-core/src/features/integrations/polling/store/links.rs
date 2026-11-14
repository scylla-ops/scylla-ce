//! The SQL of the polls of the repository links: the links that are due, the GitLab webhook of a
//! link, and the moves between the webhook mode and the polling mode. Each function runs in the
//! transaction of the caller.

use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::projects::ProjectId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// The reasons of the polling mode that end when a webhook arrives.
const ENDED_BY_A_WEBHOOK: [&str; 3] = [
    "WEBHOOKS_NOT_ARRIVING",
    "WEBHOOK_DISABLED",
    "APP_HAS_NO_WEBHOOK",
];

/// Takes at most `limit` active links whose poll is due, oldest first, and moves their next poll
/// after their interval: the interval of the link in polling mode, else `safety_seconds`. A link
/// that was never polled is due `safety_seconds` after its creation, or at once in polling mode.
pub(crate) async fn claim(
    tx: &mut Tx<'_>,
    limit: i64,
    safety_seconds: f64,
) -> Result<Vec<ProjectId>, Error> {
    let rows = sqlx::query_scalar!(
        "UPDATE repository_links l
         SET next_poll_at = now() + make_interval(secs => CASE
             WHEN l.delivery_mode = 'polling' THEN l.poll_interval_seconds::float8 ELSE $2 END)
         FROM (SELECT d.project_id FROM repository_links d
               JOIN vcs_connections c ON c.id = d.connection_id
               WHERE COALESCE(d.next_poll_at, d.created_at + make_interval(secs => $2)) <= now()
                 AND d.state = 'active' AND c.state = 'active'
               ORDER BY COALESCE(d.next_poll_at, d.created_at)
               LIMIT $1
               FOR UPDATE OF d SKIP LOCKED) due
         WHERE l.project_id = due.project_id
         RETURNING l.project_id",
        limit,
        safety_seconds,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(ProjectId::from_db).collect())
}

/// Returns the id of the GitLab webhook of the link of a project, or `None`.
pub(crate) async fn hook_id(tx: &mut Tx<'_>, project: ProjectId) -> Result<Option<String>, Error> {
    let hook = sqlx::query_scalar!(
        "SELECT hook_id FROM repository_links WHERE project_id = $1",
        project.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(hook.flatten())
}

/// Moves the next poll of the link of a project to a time, after a limit of the provider.
pub(crate) async fn delay(
    tx: &mut Tx<'_>,
    project: ProjectId,
    until: OffsetDateTime,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE repository_links SET next_poll_at = GREATEST(next_poll_at, $2)
         WHERE project_id = $1",
        project.to_db(),
        until,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Moves the active link of a project in webhook mode to the polling mode with a reason, with
/// its next poll after its interval. Returns its connection, or `None` when it did not move.
pub(crate) async fn to_polling(
    tx: &mut Tx<'_>,
    project: ProjectId,
    reason: &str,
) -> Result<Option<VcsConnectionId>, Error> {
    let connection = sqlx::query_scalar!(
        "UPDATE repository_links
         SET delivery_mode = 'polling', state_reason = $2, updated_at = now(),
             next_poll_at = now() + make_interval(secs => poll_interval_seconds::float8)
         WHERE project_id = $1 AND delivery_mode = 'webhook' AND state = 'active'
         RETURNING connection_id",
        project.to_db(),
        reason,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(connection.map(VcsConnectionId::from_db))
}

/// Moves the active link of a project to the polling mode with a reason, from the webhook mode,
/// or from the polling mode with no reason or a reason that a webhook ends. A link that moves
/// from the webhook mode polls at once. Returns its connection, or `None` when it did not move.
pub(crate) async fn to_polling_for(
    tx: &mut Tx<'_>,
    project: ProjectId,
    reason: &str,
) -> Result<Option<VcsConnectionId>, Error> {
    let connection = sqlx::query_scalar!(
        "UPDATE repository_links
         SET delivery_mode = 'polling', state_reason = $2, updated_at = now(),
             next_poll_at = CASE WHEN delivery_mode = 'webhook' THEN now() ELSE next_poll_at END
         WHERE project_id = $1 AND state = 'active'
           AND (delivery_mode = 'webhook' OR state_reason IS NULL OR state_reason = ANY($3))
         RETURNING connection_id",
        project.to_db(),
        reason,
        &ENDED_BY_A_WEBHOOK.map(str::to_owned),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(connection.map(VcsConnectionId::from_db))
}

/// Moves the active link of a project in polling mode back to the webhook mode, with its next
/// poll after `safety_seconds`, unless its reason is one that a webhook does not end. Returns
/// `true` when it moved.
pub(crate) async fn to_webhook(
    tx: &mut Tx<'_>,
    project: ProjectId,
    safety_seconds: f64,
) -> Result<bool, Error> {
    let moved = sqlx::query!(
        "UPDATE repository_links
         SET delivery_mode = 'webhook', state_reason = NULL, updated_at = now(),
             next_poll_at = now() + make_interval(secs => $2)
         WHERE project_id = $1 AND delivery_mode = 'polling' AND state = 'active'
           AND (state_reason IS NULL OR state_reason = ANY($3))",
        project.to_db(),
        safety_seconds,
        &ENDED_BY_A_WEBHOOK.map(str::to_owned),
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(moved > 0)
}

/// Moves a connection to the polling mode: its new links poll from the start.
pub(crate) async fn connection_to_polling(
    tx: &mut Tx<'_>,
    connection: VcsConnectionId,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE vcs_connections SET delivery_mode = 'polling', updated_at = now() WHERE id = $1",
        connection.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
