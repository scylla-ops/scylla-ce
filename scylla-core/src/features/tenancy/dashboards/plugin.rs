//! The plugin of the saved layouts of the dashboard.

use scylla_domain::tenancy::dashboards::{DashboardCaller, DashboardHolder};
use scylla_extension::{EditionBuilder, Plugin};

/// Adds the permission model of the dashboards. Install it after `Workspaces`: it extends
/// `Workspace`. The server adds `DashboardService`, as it reads the open streams of the agents.
pub(crate) struct Dashboards;

impl Plugin for Dashboards {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition
            .authz::<DashboardHolder>()
            .authz::<DashboardCaller>()
    }
}
