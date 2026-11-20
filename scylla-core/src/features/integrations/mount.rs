//! The mount of the Git integration: each part adds its gRPC services, its public HTTP routes
//! and its background tasks to the server.

use axum::Router;
use tokio::task::JoinHandle;
use tonic::service::RoutesBuilder;

use super::{
    apps, checkout, connections, events, files, links, merge_blocks, polling, reports, sources,
    IntegrationContext,
};

/// What one part adds to the server, next to the gRPC services that its `mount` adds to the
/// builder: public HTTP routes and background tasks.
#[derive(Default)]
pub(crate) struct Part {
    /// The public HTTP routes, such as the webhooks of the providers.
    pub(crate) router: Router,
    /// The background tasks, such as the sender of the reports.
    pub(crate) tasks: Vec<JoinHandle<()>>,
}

/// The background tasks of the integration.
pub(crate) struct Tasks(Vec<JoinHandle<()>>);

/// Mounts each part: its services go to the builder. Returns the public routes and the tasks.
pub(crate) fn mount(ctx: &IntegrationContext, services: &mut RoutesBuilder) -> (Router, Tasks) {
    // Mount each part.
    let parts = [
        apps::mount(ctx, services),
        connections::mount(ctx, services),
        links::mount(ctx, services),
        sources::mount(ctx, services),
        events::mount(ctx, services),
        polling::mount(ctx, services),
        reports::mount(ctx, services),
        checkout::mount(ctx, services),
        merge_blocks::mount(ctx, services),
        files::mount(ctx, services),
    ];

    // Join their routes and their tasks.
    let mut router = Router::new();
    let mut tasks = Vec::new();
    for part in parts {
        router = router.merge(part.router);
        tasks.extend(part.tasks);
    }
    (router, Tasks(tasks))
}

impl Tasks {
    /// Stops each task.
    pub(crate) fn stop(&self) {
        for task in &self.0 {
            task.abort();
        }
    }
}
