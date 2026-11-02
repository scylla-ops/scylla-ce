//! The plugin of the capabilities feature.

use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds `CapabilityService`.
pub(crate) struct Capabilities;

impl Plugin for Capabilities {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.service(|routes, ctx| {
            routes.add_service(grpc::server(ctx.edition_name(), ctx.capabilities()));
        })
    }
}
