//! The SQL of the `dashboards` table. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::dashboards::{
    Dashboard, DashboardChange, DashboardId, DashboardName, ListDashboards, NewDashboard,
    Visibility, Widgets,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// The foreign key from a dashboard to its workspace.
const WORKSPACE_FOREIGN_KEY: &str = "dashboards_workspace_id_fkey";

/// Inserts a dashboard and returns it. An unknown workspace gives `NotFound("workspace")`.
pub(crate) async fn insert(tx: &mut Tx<'_>, new: &NewDashboard) -> Result<Dashboard, Error> {
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO dashboards
             (id, workspace_id, created_by, name, visibility, schema_version, widgets)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, workspace_id, created_by, name, visibility, team_default, version,
                   created_at, updated_at",
        DashboardId::generate().to_db(),
        new.workspace_id.to_db(),
        new.created_by.to_db(),
        new.name.as_str(),
        new.visibility.as_str(),
        i32::from(new.widgets.schema_version),
        &new.widgets.json,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(WORKSPACE_FOREIGN_KEY) => {
            Error::NotFound("workspace")
        }
        error => error.into(),
    })?;
    Ok(row.into_dashboard())
}

/// Returns a dashboard with no widgets, or `None` if no dashboard has this id. `lock` locks its
/// row until the end of the transaction.
pub(crate) async fn find(
    tx: &mut Tx<'_>,
    id: DashboardId,
    lock: bool,
) -> Result<Option<Dashboard>, sqlx::Error> {
    let row = if lock {
        sqlx::query_as!(
            Row,
            "SELECT id, workspace_id, created_by, name, visibility, team_default, version,
                    created_at, updated_at
             FROM dashboards WHERE id = $1 FOR UPDATE",
            id.to_db(),
        )
        .fetch_optional(&mut **tx)
        .await?
    } else {
        sqlx::query_as!(
            Row,
            "SELECT id, workspace_id, created_by, name, visibility, team_default, version,
                    created_at, updated_at
             FROM dashboards WHERE id = $1",
            id.to_db(),
        )
        .fetch_optional(&mut **tx)
        .await?
    };
    Ok(row.map(Row::into_dashboard))
}

/// Returns the widgets of a dashboard, or `None` if no dashboard has this id.
pub(crate) async fn widgets(
    tx: &mut Tx<'_>,
    id: DashboardId,
) -> Result<Option<Widgets>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT schema_version, widgets FROM dashboards WHERE id = $1",
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| Widgets {
        schema_version: u16::try_from(row.schema_version).unwrap_or(u16::MAX),
        json: row.widgets,
    }))
}

/// Returns the dashboards of a workspace that the caller sees, after the page id, in id order,
/// with one extra row: the shared dashboards and the private dashboards of the caller.
pub(crate) async fn list(
    tx: &mut Tx<'_>,
    query: &ListDashboards,
) -> Result<Vec<Dashboard>, sqlx::Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, created_by, name, visibility, team_default, version,
                created_at, updated_at
         FROM dashboards
         WHERE workspace_id = $1
           AND (visibility = 'workspace' OR created_by = $2)
           AND ($3::uuid IS NULL OR id > $3)
         ORDER BY id
         LIMIT $4",
        query.workspace_id.to_db(),
        query.caller.to_db(),
        query.page.after.map(DashboardId::to_db),
        query.page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_dashboard).collect())
}

/// Changes a locked dashboard and gives it one more version. A private dashboard is not the team
/// default.
pub(crate) async fn update(
    tx: &mut Tx<'_>,
    id: DashboardId,
    change: &DashboardChange,
) -> Result<Dashboard, sqlx::Error> {
    let widgets = change.widgets.as_ref();
    let row = sqlx::query_as!(
        Row,
        "UPDATE dashboards
         SET name = coalesce($2, name),
             visibility = coalesce($3, visibility),
             team_default = team_default AND coalesce($3, visibility) = 'workspace',
             schema_version = coalesce($4, schema_version),
             widgets = coalesce($5, widgets),
             version = version + 1,
             updated_at = now()
         WHERE id = $1
         RETURNING id, workspace_id, created_by, name, visibility, team_default, version,
                   created_at, updated_at",
        id.to_db(),
        change.name.as_ref().map(DashboardName::as_str),
        change.visibility.map(Visibility::as_str),
        widgets.map(|widgets| i32::from(widgets.schema_version)),
        widgets.map(|widgets| &widgets.json),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.into_dashboard())
}

/// Deletes a dashboard. Returns `false` if no dashboard has this id.
pub(crate) async fn delete(tx: &mut Tx<'_>, id: DashboardId) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!("DELETE FROM dashboards WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Locks the row of a workspace, so that two changes of its team default run one after the
/// other. Returns `false` if no workspace has this id.
pub(crate) async fn lock_workspace(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_scalar!(
        "SELECT id FROM workspaces WHERE id = $1 FOR NO KEY UPDATE",
        workspace.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.is_some())
}

/// Removes the team default of a workspace.
pub(crate) async fn clear_team_default(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE dashboards SET team_default = false WHERE workspace_id = $1 AND team_default",
        workspace.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Makes a shared dashboard of a workspace its team default. Returns `None` if the workspace has
/// no shared dashboard with this id.
pub(crate) async fn set_team_default(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    id: DashboardId,
) -> Result<Option<Dashboard>, sqlx::Error> {
    let row = sqlx::query_as!(
        Row,
        "UPDATE dashboards SET team_default = true
         WHERE id = $1 AND workspace_id = $2 AND visibility = 'workspace'
         RETURNING id, workspace_id, created_by, name, visibility, team_default, version,
                   created_at, updated_at",
        id.to_db(),
        workspace.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_dashboard))
}

/// A row of the `dashboards` table, with no widgets.
struct Row {
    /// The id.
    id: Uuid,
    /// The workspace.
    workspace_id: Uuid,
    /// The creator.
    created_by: Uuid,
    /// The display name.
    name: String,
    /// The visibility: `private` or `workspace`.
    visibility: String,
    /// `true` for the team default.
    team_default: bool,
    /// The version.
    version: i64,
    /// The creation time.
    created_at: OffsetDateTime,
    /// The time of the last change.
    updated_at: OffsetDateTime,
}

impl Row {
    /// Makes the dashboard of a row.
    fn into_dashboard(self) -> Dashboard {
        Dashboard {
            id: DashboardId::from_db(self.id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            created_by: ActorId::from_db(self.created_by),
            name: self.name,
            visibility: Visibility::of_db(&self.visibility),
            team_default: self.team_default,
            version: self.version,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}
