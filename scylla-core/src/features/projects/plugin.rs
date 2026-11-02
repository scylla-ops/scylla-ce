//! The plugin of the projects feature.

use scylla_domain::projects::{Project, ProjectCaller, ProjectHolder};
use scylla_extension::{EditionBuilder, Plugin};

use super::{grpc, service};

/// Adds the permission model of the projects, their roles, and `ProjectService`. The table comes
/// from the core migrations.
pub(crate) struct Projects;

impl Plugin for Projects {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition
            .authz::<Project>()
            .authz::<ProjectHolder>()
            .authz::<ProjectCaller>()
            .grants(
                Project::GRANT,
                Project::REVOKE,
                Some(service::find_workspace),
            )
            .role(Project::ADMIN)
            .role(Project::EDITOR)
            .role(Project::VIEWER)
            .service(|routes, ctx| {
                routes.add_service(grpc::server(ctx.pool(), ctx.hooks(), ctx.gate()));
            })
    }
}
