//! The plugin of the Apps and of the agents.

use scylla_domain::identity::agents::{AgentAccess, AgentCaller, AgentHolder};
use scylla_domain::identity::apps::{App, AppHolder};
use scylla_extension::{EditionBuilder, Plugin};

/// Adds the permission model of the Apps and of the routes of the agents, and the role App
/// creator of a workspace. `AppService` and `AgentService` share the open streams with the jobs,
/// so the server adds them. The tables come from the core migrations.
pub(crate) struct Apps;

impl Plugin for Apps {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition
            .authz::<App>()
            .authz::<AppHolder>()
            .role(AppHolder::APP_CREATOR)
            .authz::<AgentCaller>()
            .authz::<AgentHolder>()
            .authz::<AgentAccess>()
    }
}
