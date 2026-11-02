//! The plugin of the workspaces feature.

use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceCaller};
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds the permission model of the workspaces, their roles, and `WorkspaceService`. The table
/// comes from the core migrations. Install it before the features that extend `Workspace`.
pub(crate) struct Workspaces;

impl Plugin for Workspaces {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition
            .authz::<Workspace>()
            .authz::<WorkspaceCaller>()
            .grants(Workspace::GRANT, Workspace::REVOKE, None)
            .role(Workspace::ADMIN)
            .role(Workspace::MEMBER)
            .role(Workspace::VIEWER)
            .service(|routes, ctx| {
                routes.add_service(grpc::server(ctx.pool(), ctx.gate()));
            })
    }
}
