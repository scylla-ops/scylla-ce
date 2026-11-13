//! The runs of the Git events: the pipelines of a linked project whose Git triggers match a push,
//! a pull request or a tag. They come with the runs from the events: today an event starts no run.

use scylla_domain::integrations::events::GitEvent;

use crate::features::integrations::links::store::Linked;
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// Starts the runs of a Git event for one active link. It starts no run yet.
pub(super) async fn handle(
    _ctx: &IntegrationContext,
    _linked: &Linked,
    _event: &GitEvent,
) -> Result<(), Error> {
    Ok(())
}
