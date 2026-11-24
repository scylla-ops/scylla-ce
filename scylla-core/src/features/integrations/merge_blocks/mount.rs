//! The mount of the rules that block a merge when a pipeline fails: `MergeBlockService`.

use tonic::service::RoutesBuilder;

use super::grpc;
use crate::features::integrations::{IntegrationContext, Part};

/// Adds `MergeBlockService`. This part has no public route and no task.
pub(crate) fn mount(ctx: &IntegrationContext, services: &mut RoutesBuilder) -> Part {
    services.add_service(grpc::server(ctx.clone()));
    Part::default()
}
