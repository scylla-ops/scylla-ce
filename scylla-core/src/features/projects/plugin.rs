//! The plugin of the projects feature.

use scylla_domain::projects::Project;
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds the permission model of the projects and `ProjectService`. The table comes from the
/// core migrations.
pub(crate) struct Projects;

impl Plugin for Projects {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.authz::<Project>().service(|routes, ctx| {
            routes.add_service(grpc::server(ctx.pool(), ctx.hooks(), ctx.gate()));
        })
    }
}
