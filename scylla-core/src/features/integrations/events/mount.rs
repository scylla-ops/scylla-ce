//! The mount of the webhooks of the providers: their public routes, and the task that deletes
//! the old deliveries.

use tonic::service::RoutesBuilder;

use super::{prune, receive};
use crate::features::integrations::{IntegrationContext, Part};

/// Adds the routes of the webhooks and the cleanup of the deliveries. This part has no gRPC
/// service.
pub(crate) fn mount(ctx: &IntegrationContext, _services: &mut RoutesBuilder) -> Part {
    Part {
        router: receive::router(ctx.clone()),
        tasks: vec![prune::spawn(ctx.clone())],
    }
}
