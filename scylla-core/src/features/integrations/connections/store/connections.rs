//! The reads and the delete of the `vcs_connections` table, and the links of a connection. Each
//! function runs in the transaction of the caller.

use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::Tx;

use super::row::{Counted, Row};
use crate::Error;

/// Returns a connection and the number of its projects, or `None` if no connection has this id.
pub(crate) async fn find(tx: &mut Tx<'_>, id: VcsConnectionId) -> Result<Option<Counted>, Error> {
    let row = sqlx::query_as!(
        Row,
        r#"SELECT c.id, c.workspace_id, c.provider, c.auth_kind, c.app_id, c.external_id, c.account,
                  c.account_kind, c.avatar_url, c.token_expires_at, c.scopes, c.permissions,
                  c.all_repositories, c.repository_count, c.delivery_mode,
                  c.poll_interval_seconds, c.last_event_at, c.state, c.state_reason, c.created_by,
                  c.created_at,
                  (SELECT count(*) FROM repository_links l WHERE l.connection_id = c.id)
                      AS "project_count!"
           FROM vcs_connections c WHERE c.id = $1"#,
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    row.map(Row::into_counted).transpose()
}

/// Returns the connections of a workspace after the id of the page, oldest first, with one extra
/// row.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    page: &PageRequest<VcsConnectionId>,
) -> Result<Vec<Counted>, Error> {
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT c.id, c.workspace_id, c.provider, c.auth_kind, c.app_id, c.external_id, c.account,
                  c.account_kind, c.avatar_url, c.token_expires_at, c.scopes, c.permissions,
                  c.all_repositories, c.repository_count, c.delivery_mode,
                  c.poll_interval_seconds, c.last_event_at, c.state, c.state_reason, c.created_by,
                  c.created_at,
                  (SELECT count(*) FROM repository_links l WHERE l.connection_id = c.id)
                      AS "project_count!"
           FROM vcs_connections c
           WHERE c.workspace_id = $1 AND ($2::uuid IS NULL OR c.id > $2)
           ORDER BY c.id
           LIMIT $3"#,
        workspace.to_db(),
        page.after.map(VcsConnectionId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter().map(Row::into_counted).collect()
}

/// Deletes a connection, with the links of its repositories. Returns `false` if no connection
/// has this id.
pub(crate) async fn delete(tx: &mut Tx<'_>, id: VcsConnectionId) -> Result<bool, Error> {
    let deleted = sqlx::query!("DELETE FROM vcs_connections WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?
        .rows_affected();
    Ok(deleted > 0)
}

/// Returns the sealed token of a connection: `None` for an unknown id or a connection with no
/// token.
pub(crate) async fn sealed_token(
    tx: &mut Tx<'_>,
    id: VcsConnectionId,
) -> Result<Option<Vec<u8>>, Error> {
    let sealed = sqlx::query_scalar!(
        "SELECT sealed_token FROM vcs_connections WHERE id = $1",
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(sealed.flatten())
}

/// Returns the webhooks that Scylla made for the links of a connection: the id of each
/// repository on the provider and the id of its hook.
pub(crate) async fn hooks(
    tx: &mut Tx<'_>,
    id: VcsConnectionId,
) -> Result<Vec<(String, String)>, Error> {
    let rows = sqlx::query!(
        r#"SELECT external_id, hook_id AS "hook_id!" FROM repository_links
           WHERE connection_id = $1 AND hook_id IS NOT NULL
           ORDER BY project_id"#,
        id.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.external_id, row.hook_id))
        .collect())
}

/// Returns the links of a connection: the id of each repository on the provider and its
/// project.
pub(crate) async fn links(
    tx: &mut Tx<'_>,
    id: VcsConnectionId,
) -> Result<Vec<(String, ProjectId)>, Error> {
    let rows = sqlx::query!(
        "SELECT external_id, project_id FROM repository_links WHERE connection_id = $1",
        id.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (row.external_id, ProjectId::from_db(row.project_id)))
        .collect())
}

/// Returns the workspace of a project, or `None` if no project has this id.
pub(crate) async fn project_workspace(
    tx: &mut Tx<'_>,
    project: ProjectId,
) -> Result<Option<WorkspaceId>, Error> {
    let workspace = sqlx::query_scalar!(
        "SELECT workspace_id FROM projects WHERE id = $1",
        project.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(workspace.map(WorkspaceId::from_db))
}
