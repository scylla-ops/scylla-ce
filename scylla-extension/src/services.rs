use sqlx::PgPool;
use tonic::service::RoutesBuilder;

pub trait ServiceProvider: Send + Sync + 'static {
    fn add_services(&self, routes: &mut RoutesBuilder, pool: &PgPool);
}

pub struct NoExtraServices;

impl ServiceProvider for NoExtraServices {
    fn add_services(&self, _routes: &mut RoutesBuilder, _pool: &PgPool) {}
}
