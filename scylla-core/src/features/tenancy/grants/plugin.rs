//! The plugin of the grants feature.

use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds `AccessService`. It gives only the roles of the edition registry.
pub(crate) struct Grants;

impl Plugin for Grants {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.service(|routes, ctx| {
            routes.add_service(grpc::server(ctx.pool(), ctx.gate(), ctx.grants()));
        })
    }
}
