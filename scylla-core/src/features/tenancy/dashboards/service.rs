//! The logic of the saved layouts of the dashboard: one transaction for each call, with the
//! hooks.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::tenancy::dashboards::{
    Dashboard, DashboardChange, DashboardId, DashboardPage, ListDashboards, NewDashboard, Widgets,
    KIND,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::{CreateContext, DeleteContext, Hooks};
use sqlx::PgPool;

use super::store::dashboards as store;
use crate::Error;

/// Creates a dashboard if each `BeforeCreate` hook accepts it.
pub(super) async fn create(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    new: NewDashboard,
) -> Result<Dashboard, Error> {
    let mut tx = pool.begin().await?;
    hooks
        .before_create(&mut tx, &CreateContext::new(KIND, actor, new.workspace_id))
        .await?;
    let dashboard = store::insert(&mut tx, &new).await?;
    tx.commit().await?;
    Ok(dashboard)
}

/// Returns a dashboard with no widgets, or `None` if no dashboard has this id.
pub(super) async fn find(pool: &PgPool, id: DashboardId) -> Result<Option<Dashboard>, Error> {
    let mut tx = pool.begin().await?;
    let dashboard = store::find(&mut tx, id, false).await?;
    tx.commit().await?;
    Ok(dashboard)
}

/// Returns a dashboard with its widgets, or `None` if no dashboard has this id.
pub(super) async fn get(
    pool: &PgPool,
    id: DashboardId,
) -> Result<Option<(Dashboard, Widgets)>, Error> {
    let mut tx = pool.begin().await?;
    let dashboard = store::find(&mut tx, id, false).await?;
    let widgets = store::widgets(&mut tx, id).await?;
    tx.commit().await?;
    Ok(dashboard.zip(widgets))
}

/// Returns one page of the dashboards of a workspace that the caller sees, oldest first.
pub(super) async fn list(pool: &PgPool, query: ListDashboards) -> Result<DashboardPage, Error> {
    let mut tx = pool.begin().await?;
    let rows = store::list(&mut tx, &query).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(rows, query.page.size, |dashboard| {
        dashboard.id
    }))
}

/// Changes a dashboard of the version that the client read. Another version gives `ABORTED`. A
/// dashboard that the caller cannot see any more gives `NotFound`.
pub(super) async fn update(
    pool: &PgPool,
    caller: ActorId,
    id: DashboardId,
    change: DashboardChange,
) -> Result<Dashboard, Error> {
    // Lock the dashboard, then refuse a dashboard that went away or became private.
    let mut tx = pool.begin().await?;
    let current = store::find(&mut tx, id, true)
        .await?
        .filter(|dashboard| !dashboard.is_hidden_from(caller))
        .ok_or(Error::NotFound("dashboard"))?;

    // Refuse a stale version, then change the dashboard.
    if current.version != change.version {
        return Err(Error::Conflict("the dashboard changed: read it again"));
    }
    let dashboard = store::update(&mut tx, id, &change).await?;
    tx.commit().await?;
    Ok(dashboard)
}

/// Deletes a dashboard and runs the `AfterDelete` hooks. A dashboard that the caller cannot see
/// any more gives `NotFound`.
pub(super) async fn delete(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    id: DashboardId,
) -> Result<(), Error> {
    // Lock the dashboard, then refuse a dashboard that went away or became private.
    let mut tx = pool.begin().await?;
    let dashboard = store::find(&mut tx, id, true)
        .await?
        .filter(|dashboard| !dashboard.is_hidden_from(actor.id))
        .ok_or(Error::NotFound("dashboard"))?;

    // Delete the row and run the hooks in one transaction.
    store::delete(&mut tx, id).await?;
    let context = DeleteContext::new(KIND, actor, dashboard.workspace_id);
    hooks.after_delete(&mut tx, &context).await?;
    tx.commit().await?;
    Ok(())
}

/// Makes a shared dashboard the team default of its workspace, or removes the team default when
/// no dashboard is given. Returns the new team default.
pub(super) async fn set_team_default(
    pool: &PgPool,
    workspace: WorkspaceId,
    id: Option<DashboardId>,
) -> Result<Option<Dashboard>, Error> {
    // Lock the workspace, so that two changes of its team default run one after the other.
    let mut tx = pool.begin().await?;
    if !store::lock_workspace(&mut tx, workspace).await? {
        return Err(Error::NotFound("workspace"));
    }

    // Remove the team default, then set the new one.
    store::clear_team_default(&mut tx, workspace).await?;
    let dashboard = match id {
        Some(id) => Some(
            store::set_team_default(&mut tx, workspace, id)
                .await?
                .ok_or(Error::NotFound("dashboard"))?,
        ),
        None => None,
    };
    tx.commit().await?;
    Ok(dashboard)
}
