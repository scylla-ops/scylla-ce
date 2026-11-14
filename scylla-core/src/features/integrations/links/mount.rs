//! The mount of the repository link of a project: `RepositoryService`.

use tonic::service::RoutesBuilder;

use super::grpc;
use crate::features::integrations::{IntegrationContext, Part};

/// Adds `RepositoryService`. This part has no public route and no task.
pub(crate) fn mount(ctx: &IntegrationContext, services: &mut RoutesBuilder) -> Part {
    services.add_service(grpc::server(ctx.clone()));
    Part::default()
}
