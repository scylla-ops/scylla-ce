//! The mount of the GitHub App of an instance, made with the manifest flow: `VcsAppService`.

use tonic::service::RoutesBuilder;

use super::grpc;
use crate::features::integrations::{IntegrationContext, Part};

/// Adds `VcsAppService`. This part has no public route and no task.
pub(crate) fn mount(ctx: &IntegrationContext, services: &mut RoutesBuilder) -> Part {
    services.add_service(grpc::server(ctx.clone()));
    Part::default()
}
