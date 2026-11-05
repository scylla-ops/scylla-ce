//! The plugin of the workspaces feature.

use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceCaller};
use scylla_extension::{AccessRoutes, EditionBuilder, Plugin};

use super::{grpc, service};

/// Adds the permission model of the workspaces, their roles, and `WorkspaceService`. The table
/// comes from the core migrations. Install it before the features that extend `Workspace`.
pub(crate) struct Workspaces;

impl Plugin for Workspaces {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        let routes = AccessRoutes {
            grant: Workspace::GRANT,
            revoke: Workspace::REVOKE,
            list_access: Workspace::LIST_ACCESS,
        };
        edition
            .authz::<Workspace>()
            .authz::<WorkspaceCaller>()
            .grants(routes, service::find_workspace)
            .role(Workspace::ADMIN)
            .role(Workspace::MEMBER)
            .role(Workspace::VIEWER)
            .service(|routes, ctx| {
                routes.add_service(grpc::server(ctx.pool(), ctx.gate(), ctx.hooks()));
            })
    }
}
