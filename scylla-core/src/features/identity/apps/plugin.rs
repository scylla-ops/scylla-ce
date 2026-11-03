//! The plugin of the Apps and of the agents.

use std::sync::Arc;

use scylla_domain::identity::agents::AgentCaller;
use scylla_domain::identity::apps::{App, AppHolder};
use scylla_extension::{EditionBuilder, Plugin};

use super::grpc;
use crate::features::identity::agents::{self, Registry};

/// Adds the permission model of the Apps, `AppService`, and `AgentService` for the streams of
/// the agents. The two services share the registry of the open streams. The tables come from
/// the core migrations.
pub(crate) struct Apps;

impl Plugin for Apps {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        let registry = Arc::new(Registry::default());
        edition
            .authz::<App>()
            .authz::<AppHolder>()
            .authz::<AgentCaller>()
            .service(move |routes, ctx| {
                let (pool, gate) = (ctx.pool(), ctx.gate());
                let apps = grpc::server(pool.clone(), Arc::clone(&gate), Arc::clone(&registry));
                routes.add_service(apps);
                routes.add_service(agents::server(pool, gate, Arc::clone(&registry)));
            })
    }
}
