//! The plugin of the saved layouts of the dashboard.

use scylla_domain::tenancy::dashboards::{DashboardCaller, DashboardHolder};
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds the permission model of the dashboards and `DashboardService`. Install it after
/// `Workspaces`: it extends `Workspace`.
pub(crate) struct Dashboards;

impl Plugin for Dashboards {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition
            .authz::<DashboardHolder>()
            .authz::<DashboardCaller>()
            .service(|routes, ctx| {
                routes.add_service(grpc::server(ctx.pool(), ctx.hooks(), ctx.gate()));
            })
    }
}
