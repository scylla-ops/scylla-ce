//! Order: `hooks` before the server builds `Context::actions`, `prepare` before anything serves,
//! `install` after the strata edition is built.

use crate::surface::Surface;
use async_trait::async_trait;
use scylla_auth::authz::{PermissionService, VisibilityResolver};
use scylla_core::domain::caller::CallerContext;
use scylla_core::domain::errors::DomainResult;
use scylla_core::domain::permission::Permission;
use scylla_extension::{Actions, Authorizer, Hooks};
use sqlx::PgPool;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type PrepareFuture<'a> = Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;

/// `actions` and `Feature::hooks` serve only the Actions features of an out-of-tree edition: the
/// CE slices run on strata and run no hook. `permissions` is the access model of the CE;
/// `visibility` serves a `Fetch` scope. `trust_forwarded_headers` is
/// `[server].trust_forwarded_headers`, for `scylla_core::grpc::session_client` in a handler that
/// opens a session.
#[derive(Clone)]
pub struct Context {
    pub db: PgPool,
    pub actions: Arc<Actions>,
    pub permissions: Arc<dyn PermissionService>,
    pub visibility: Arc<dyn VisibilityResolver>,
    pub trust_forwarded_headers: bool,
}

pub trait Feature: Send + 'static {
    fn hooks(&self, _hooks: &mut Hooks) {}

    fn prepare<'a>(&'a self, _db: &'a PgPool) -> PrepareFuture<'a> {
        Box::pin(async { Ok(()) })
    }

    fn install(self: Box<Self>, _ctx: &Context, _surface: &mut Surface) {}
}

/// The authorizer of `Context::actions`: the permission check of the access model.
pub(crate) struct PermissionAuthorizer(pub Arc<dyn PermissionService>);

#[async_trait]
impl Authorizer for PermissionAuthorizer {
    async fn authorize(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        self.0.check(caller, permission).await
    }
}
