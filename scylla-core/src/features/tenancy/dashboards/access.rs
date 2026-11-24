//! The check of a route permission on one dashboard: its creator, or a role of its workspace.

use scylla_domain::authz::{Object, Permission};
use scylla_domain::identity::actors::Actor;
use scylla_domain::tenancy::dashboards::{
    Dashboard, DashboardCaller, DashboardHolder, DashboardId,
};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{AccessError, Gate, Principal};

use crate::Error;

/// The permissions of a route on one dashboard.
#[derive(Clone, Copy)]
pub(super) struct Route {
    /// The permission on the creator itself.
    creator: Permission<Actor>,
    /// The permission on the workspace of the dashboard.
    holder: Permission<Workspace>,
}

/// The route `GetDashboard`.
pub(super) const GET: Route = Route {
    creator: DashboardCaller::GET_DASHBOARD,
    holder: DashboardHolder::GET_DASHBOARD,
};

/// The route `UpdateDashboard`.
pub(super) const UPDATE: Route = Route {
    creator: DashboardCaller::UPDATE_DASHBOARD,
    holder: DashboardHolder::UPDATE_DASHBOARD,
};

/// The route `DeleteDashboard`.
pub(super) const DELETE: Route = Route {
    creator: DashboardCaller::DELETE_DASHBOARD,
    holder: DashboardHolder::DELETE_DASHBOARD,
};

/// Checks a route on one dashboard. Its creator passes on itself, in the workspace of the
/// dashboard, while it reads the dashboards of that workspace. Another caller needs the route
/// permission on the workspace. An unknown dashboard, a private dashboard of another actor and a
/// refusal get the same answer, which names the dashboard, not its workspace.
pub(super) async fn authorize(
    gate: &dyn Gate,
    caller: &Principal,
    route: Route,
    id: DashboardId,
    found: Option<&Dashboard>,
) -> Result<(), Error> {
    // Refuse an unknown dashboard, and a private dashboard of another actor.
    let object = format!("dashboard:{id}");
    let name = route.holder.name();
    let reason = format!("{name} on {object} is denied");
    let Some(dashboard) = found.filter(|found| !found.is_hidden_from(caller.actor.id)) else {
        return Err(AccessError::refused(caller, name, &object, reason).into());
    };

    // Check the creator that still reads the workspace, else the workspace.
    let workspace = dashboard.workspace_id;
    let creator =
        dashboard.is_created_by(caller.actor.id) && reads(gate, caller, workspace).await?;
    let check = if creator {
        route.creator.on(caller.actor.id).within(workspace)
    } else {
        route.holder.on(workspace)
    };

    // Hide the workspace in a refusal.
    match gate.authorize(caller, check).await {
        Ok(()) => Ok(()),
        Err(AccessError::Denied(_)) => Err(AccessError::Denied(reason).into()),
        Err(error) => Err(error.into()),
    }
}

/// Returns `true` if the caller reads the dashboards of a workspace. It is not a route check.
async fn reads(gate: &dyn Gate, caller: &Principal, workspace: WorkspaceId) -> Result<bool, Error> {
    let names = [DashboardHolder::GET_DASHBOARD.name()];
    let answers = gate
        .permissions(caller, &Object::of(workspace), Some(workspace), &names)
        .await?;
    Ok(answers.first().copied().unwrap_or(false))
}
