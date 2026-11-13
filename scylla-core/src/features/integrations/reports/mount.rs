//! The mount of the reports of the runs on the commits: `VcsReportService` and the sender task.

use tonic::service::RoutesBuilder;

use super::{grpc, sender};
use crate::features::integrations::{IntegrationContext, Part};

/// Adds `VcsReportService`, and starts the sender of the reports. This part has no public route.
pub(crate) fn mount(ctx: &IntegrationContext, services: &mut RoutesBuilder) -> Part {
    services.add_service(grpc::server(ctx.clone()));
    Part {
        tasks: vec![sender::spawn(ctx.clone())],
        ..Part::default()
    }
}
