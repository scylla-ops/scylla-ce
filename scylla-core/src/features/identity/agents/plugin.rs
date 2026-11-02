//! The plugin of the agents feature.

use scylla_domain::identity::agents::AgentHolder;
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;

/// Adds the route permission of the agents and `AgentService`. The tables come from the core
/// migrations.
pub(crate) struct Agents;

impl Plugin for Agents {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.authz::<AgentHolder>().service(|routes, ctx| {
            routes.add_service(grpc::server(ctx.pool(), ctx.gate()));
        })
    }
}
