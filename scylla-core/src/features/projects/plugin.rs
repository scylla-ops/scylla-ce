//! The plugin of the projects feature.

use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds `ProjectService`. The table comes from the core migrations.
pub(crate) struct Projects;

impl Plugin for Projects {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.service(|routes, ctx| {
            routes.add_service(grpc::server(ctx.pool(), ctx.hooks(), ctx.authenticators()));
        })
    }
}
