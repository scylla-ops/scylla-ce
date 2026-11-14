//! The plugin of the Git integration: the kinds of the notices of the connections. Its services,
//! public routes and tasks start in `serve` through `mount`.

use scylla_extension::{EditionBuilder, Plugin};

use super::reports::notices;

/// Adds the kinds `vcs.connection_broken` and `vcs.token_expiring`.
pub(crate) struct Integrations;

impl Plugin for Integrations {
    fn install(self, edition: EditionBuilder) -> EditionBuilder {
        notices::kinds()
            .into_iter()
            .fold(edition, |edition, kind| edition.notification_kind(kind))
    }
}
