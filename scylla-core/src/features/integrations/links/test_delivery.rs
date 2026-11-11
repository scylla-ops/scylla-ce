//! The test of the webhook of a linked repository, for `TestDelivery`. The server does not serve
//! it yet: the webhooks and the polling come with the provider events.

use tonic::Status;

use super::store::Linked;
use crate::features::integrations::IntegrationContext;

/// The result of a test of the webhook of a link.
#[allow(
    dead_code,
    reason = "the test of the webhooks comes with the provider events"
)]
pub(super) struct Delivery {
    /// `true` when the test webhook arrived.
    pub(super) delivered: bool,
    /// Why the test failed, as a short text, or an empty text.
    pub(super) error: String,
}

/// Asks the provider for a test webhook of a link, and tells if it arrived. The server does not
/// serve it yet.
pub(super) async fn test(_ctx: &IntegrationContext, _linked: &Linked) -> Result<Delivery, Status> {
    Err(Status::unimplemented("TestDelivery is not served"))
}
