//! The plugin of the projects feature.

use scylla_domain::projects::{Project, ProjectCaller, ProjectHolder};
use scylla_extension::{AccessRoutes, EditionBuilder, Plugin};

use super::{grpc, service};

/// Adds the permission model of the projects, their roles, and `ProjectService`. The table comes
/// from the core migrations.
pub(crate) struct Projects;

impl Plugin for Projects {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        let routes = AccessRoutes {
            grant: Project::GRANT,
            revoke: Project::REVOKE,
            list_access: Project::LIST_ACCESS,
        };
        edition
            .authz::<Project>()
            .authz::<ProjectHolder>()
            .authz::<ProjectCaller>()
            .grants(routes, service::find_workspace)
            .role(Project::ADMIN)
            .role(Project::EDITOR)
            .role(Project::VIEWER)
            .service(|routes, ctx| {
                routes.add_service(grpc::server(ctx.pool(), ctx.hooks(), ctx.gate()));
            })
    }
}
