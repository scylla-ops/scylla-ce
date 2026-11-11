//! The mount of the webhooks of the providers, and the runs of their events. It adds nothing yet.

use tonic::service::RoutesBuilder;

use crate::features::integrations::{IntegrationContext, Part};

/// Adds the services, the routes and the tasks of this part. It adds nothing yet.
pub(crate) fn mount(_ctx: &IntegrationContext, _services: &mut RoutesBuilder) -> Part {
    Part::default()
}
