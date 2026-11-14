//! The mount of the polling of the linked repositories: its task.

use tonic::service::RoutesBuilder;

use super::task;
use crate::features::integrations::{IntegrationContext, Part};

/// Adds the task of the polls. This part has no gRPC service and no public route.
pub(crate) fn mount(ctx: &IntegrationContext, _services: &mut RoutesBuilder) -> Part {
    Part {
        tasks: vec![task::spawn(ctx.clone())],
        ..Part::default()
    }
}
