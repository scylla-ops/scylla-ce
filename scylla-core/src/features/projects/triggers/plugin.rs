//! The plugin of the triggers.

use scylla_domain::projects::triggers::{TriggerAccess, TriggerHolder};
use scylla_extension::{EditionBuilder, Plugin};

/// Adds the permission model of the triggers. The service, the webhook route and the loop of the
/// schedules need the open streams of the agents and the key of the secrets, so the server adds
/// them. The tables come from the core migrations.
pub(crate) struct Triggers;

impl Plugin for Triggers {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        edition.authz::<TriggerAccess>().authz::<TriggerHolder>()
    }
}
