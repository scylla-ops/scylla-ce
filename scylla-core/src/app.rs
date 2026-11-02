use std::sync::Arc;

use scylla_extension::{CapabilityProvider, ResourceGuard, ServiceProvider};
use sqlx::migrate::Migrator;
use sqlx::PgPool;
use tonic::service::{Routes, RoutesBuilder};

use crate::features::{capabilities, projects};

#[derive(Clone)]
pub struct Edition {
    pub guard: Arc<dyn ResourceGuard>,
    pub capabilities: Arc<dyn CapabilityProvider>,
    pub services: Arc<dyn ServiceProvider>,
    pub migrations: &'static [&'static Migrator],
}

pub fn routes(pool: PgPool, edition: &Edition) -> Routes {
    let mut routes = RoutesBuilder::default();
    routes
        .add_service(projects::server(pool.clone(), edition.guard.clone()))
        .add_service(capabilities::server(edition.capabilities.clone()));
    edition.services.add_services(&mut routes, &pool);
    routes.routes()
}
