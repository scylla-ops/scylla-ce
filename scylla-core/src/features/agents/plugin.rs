//! The plugin of the agents feature.

use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds `AgentService`. The tables come from the core migrations.
pub(crate) struct Agents;

impl Plugin for Agents {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.service(|routes, ctx| {
            routes.add_service(grpc::server(ctx.pool(), ctx.gate()));
        })
    }
}
