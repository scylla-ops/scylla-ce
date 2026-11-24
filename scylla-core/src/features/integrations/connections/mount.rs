//! The mount of the GitHub and GitLab connections of a workspace: `VcsConnectionService`.

use tonic::service::RoutesBuilder;

use super::grpc;
use crate::features::integrations::{IntegrationContext, Part};

/// Adds `VcsConnectionService`. This part has no public route and no task.
pub(crate) fn mount(ctx: &IntegrationContext, services: &mut RoutesBuilder) -> Part {
    services.add_service(grpc::server(ctx.clone()));
    Part::default()
}
