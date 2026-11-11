//! The mount of the commit context of a run, and a run on a branch, a tag or a SHA. It adds nothing yet.

use tonic::service::RoutesBuilder;

use crate::features::integrations::{IntegrationContext, Part};

/// Adds the services, the routes and the tasks of this part. It adds nothing yet.
pub(crate) fn mount(_ctx: &IntegrationContext, _services: &mut RoutesBuilder) -> Part {
    Part::default()
}
