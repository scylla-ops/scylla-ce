//! The SQL of the lists of workspaces: the console of the workspaces of the instance, and the
//! details of the workspaces of a page.

use std::collections::HashMap;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::images::{Avatar, ImageId};
use scylla_domain::tenancy::workspaces::lists::{ListAllWorkspaces, WorkspaceOwner};
use scylla_domain::tenancy::workspaces::{VisibleWorkspaces, Workspace, WorkspaceId};
use scylla_extension::Tx;
use time::OffsetDateTime;

use super::row::Row;
use crate::Error;

/// The facts of a workspace that Postgres counts.
pub(crate) struct Facts {
    /// The number of projects.
    pub(crate) projects: i64,
    /// The time of the newest entry of the activity feed.
    pub(crate) last_activity_at: Option<OffsetDateTime>,
}

/// An owner of a workspace.
pub(crate) struct Owned {
    /// The workspace.
    pub(crate) workspace: WorkspaceId,
    /// The owner, with the facts that a list shows.
    pub(crate) owner: WorkspaceOwner,
}

/// Returns the workspaces of the console after the page id, in the order of the query, with one
/// extra row, and the number of workspaces that the search keeps. `mine` tells which workspaces
/// the caller owns or has a role in.
pub(crate) async fn list_all(
    tx: &mut Tx<'_>,
    query: &ListAllWorkspaces,
    mine: &VisibleWorkspaces,
) -> Result<(Vec<Workspace>, i64), Error> {
    let granted: Vec<_> = mine.granted.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"WITH kept AS (
               SELECT w.id, w.name, w.slug, w.created_at, w.personal_tokens_enabled, w.time_zone,
                      w.icon_name, w.icon_color, w.icon_image_id,
                      row_number() OVER (ORDER BY
                          CASE WHEN $1 = 'name' THEN lower(w.name) END,
                          CASE WHEN $1 = 'name' THEN lower(coalesce(first_owner.display_name, ''))
                          END,
                          CASE WHEN $1 = 'newest' THEN w.created_at END DESC,
                          CASE WHEN $1 = 'newest' THEN w.id END DESC,
                          CASE WHEN $1 = 'activity' THEN last_activity.at END DESC NULLS LAST,
                          w.id) AS position,
                      count(*) OVER () AS total
               FROM workspaces w
               LEFT JOIN LATERAL (
                   SELECT u.display_name FROM workspace_owners o JOIN users u ON u.id = o.user_id
                   WHERE o.workspace_id = w.id
                   ORDER BY o.created_at, o.user_id
                   LIMIT 1
               ) first_owner ON true
               LEFT JOIN LATERAL (
                   SELECT max(a.created_at) AS at FROM activity a WHERE a.workspace_id = w.id
               ) last_activity ON true
               WHERE ($2 = ''
                      OR strpos(lower(w.name), $2) > 0
                      OR strpos(w.slug, $2) > 0
                      OR EXISTS (
                          SELECT 1 FROM workspace_owners o
                          JOIN users u ON u.id = o.user_id
                          LEFT JOIN user_emails e ON e.user_id = u.id AND e.is_primary
                          WHERE o.workspace_id = w.id
                            AND (strpos(lower(u.display_name), $2) > 0
                                 OR strpos(lower(coalesce(u.handle, '')), $2) > 0
                                 OR strpos(lower(coalesce(e.email, '')), $2) > 0)))
                 AND NOT ($3 AND (w.id = ANY($5)
                                  OR EXISTS (SELECT 1 FROM workspace_owners o
                                             WHERE o.workspace_id = w.id AND o.user_id = $4)))
           )
           SELECT id AS "id!", name AS "name!", slug AS "slug!", created_at AS "created_at!",
                  personal_tokens_enabled AS "personal_tokens_enabled!",
                  time_zone AS "time_zone!", icon_name, icon_color, icon_image_id,
                  total AS "total!"
           FROM kept
           WHERE position > CASE
                   WHEN $6::uuid IS NULL THEN 0
                   ELSE coalesce((SELECT k.position FROM kept k WHERE k.id = $6),
                                 9223372036854775807)
                 END
           ORDER BY position
           LIMIT $7"#,
        query.order.key(),
        query.search.as_str(),
        query.others_only,
        mine.caller.to_db(),
        &granted,
        query.page.after.map(WorkspaceId::to_db),
        query.page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    let total = rows.first().map_or(0, |row| row.total);
    let workspaces = rows
        .into_iter()
        .map(|row| {
            Row {
                id: row.id,
                name: row.name,
                slug: row.slug,
                created_at: row.created_at,
                personal_tokens_enabled: row.personal_tokens_enabled,
                time_zone: row.time_zone,
                icon_name: row.icon_name,
                icon_color: row.icon_color,
                icon_image_id: row.icon_image_id,
            }
            .into_workspace()
        })
        .collect();
    Ok((workspaces, total))
}

/// Returns the number of projects and the last activity of each workspace.
pub(crate) async fn facts(
    tx: &mut Tx<'_>,
    ids: &[WorkspaceId],
) -> Result<HashMap<WorkspaceId, Facts>, Error> {
    let ids: Vec<_> = ids.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"SELECT w.id,
                  (SELECT count(*) FROM projects p WHERE p.workspace_id = w.id) AS "projects!",
                  (SELECT max(a.created_at) FROM activity a WHERE a.workspace_id = w.id)
                      AS last_activity_at
           FROM workspaces w
           WHERE w.id = ANY($1)"#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let facts = Facts {
                projects: row.projects,
                last_activity_at: row.last_activity_at,
            };
            (WorkspaceId::from_db(row.id), facts)
        })
        .collect())
}

/// Returns the owners of the workspaces, oldest first in each workspace. `with_email` gives the
/// primary email of each owner.
pub(crate) async fn owners_of(
    tx: &mut Tx<'_>,
    ids: &[WorkspaceId],
    with_email: bool,
) -> Result<Vec<Owned>, Error> {
    let ids: Vec<_> = ids.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        r#"SELECT o.workspace_id, o.user_id, u.display_name, u.handle, e.email AS "email?",
                  u.avatar_image_id, u.avatar_color
           FROM workspace_owners o
           JOIN users u ON u.id = o.user_id
           LEFT JOIN user_emails e ON e.user_id = u.id AND e.is_primary
           WHERE o.workspace_id = ANY($1)
           ORDER BY o.workspace_id, o.created_at, o.user_id"#,
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Owned {
            workspace: WorkspaceId::from_db(row.workspace_id),
            owner: WorkspaceOwner {
                user: ActorId::from_db(row.user_id),
                display_name: row.display_name,
                handle: row.handle,
                email: row.email.filter(|_| with_email),
                avatar: Avatar::of_columns(
                    row.avatar_image_id.map(ImageId::from_db),
                    row.avatar_color,
                ),
            },
        })
        .collect())
}

/// Returns each role that is a row on the workspaces: the workspace, the actor and the name of the
/// role.
pub(crate) async fn row_roles_of(
    tx: &mut Tx<'_>,
    ids: &[WorkspaceId],
) -> Result<Vec<(WorkspaceId, ActorId, String)>, Error> {
    let ids: Vec<_> = ids.iter().map(|id| id.to_db()).collect();
    let rows = sqlx::query!(
        "SELECT g.scope_id, g.actor_id, r.name
         FROM role_grants g
         JOIN roles r ON r.id = g.role_id
         WHERE g.scope_kind = 'workspace' AND g.scope_id = ANY($1)
         ORDER BY g.scope_id, lower(r.name), g.actor_id",
        &ids,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let workspace = WorkspaceId::from_db(row.scope_id);
            (workspace, ActorId::from_db(row.actor_id), row.name)
        })
        .collect())
}
