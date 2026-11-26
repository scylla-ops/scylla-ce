//! The plugin of the groups of the projects.

use scylla_domain::projects::groups::{ProjectGroupAccess, ProjectGroupHolder};
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds the permission model of the groups of the projects and `ProjectGroupService`. Install it
/// after `Projects`: it extends `Workspace` and `Project`. The table comes from the core
/// migrations.
pub(crate) struct ProjectGroups;

impl Plugin for ProjectGroups {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition
            .authz::<ProjectGroupHolder>()
            .authz::<ProjectGroupAccess>()
            .service(|routes, ctx| {
                routes.add_service(grpc::server(ctx.pool(), ctx.hooks(), ctx.gate()));
            })
    }
}
