//! Order: `prepare` before anything serves, `install` after the strata edition is built.

use crate::surface::Surface;
use scylla_auth::authz::{PermissionService, VisibilityResolver};
use sqlx::PgPool;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type PrepareFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

/// `permissions` and `visibility` are the access model of the CE. `trust_forwarded_headers` is
/// `[server].trust_forwarded_headers`, for `scylla_core::grpc::session_client` in a handler that
/// opens a session.
#[derive(Clone)]
pub struct Context {
    pub db: PgPool,
    pub permissions: Arc<dyn PermissionService>,
    pub visibility: Arc<dyn VisibilityResolver>,
    pub trust_forwarded_headers: bool,
}

pub trait Feature: Send + 'static {
    fn prepare<'a>(&'a self, _db: &'a PgPool) -> PrepareFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn install(self: Box<Self>, _ctx: &Context, _surface: &mut Surface) {}
}
