//! The tests of the route rule: a route that verifies a token checks its own permission.

use std::sync::Arc;

use scylla_core::db::MIGRATOR;
use scylla_core::Error;
use scylla_domain::actors::Actor;
use scylla_domain::authz;
use scylla_extension::{Edition, Gate};
use scylla_proto::capabilities::v1::capability_service_client::CapabilityServiceClient;
use scylla_proto::capabilities::v1::capability_service_server::{
    CapabilityService, CapabilityServiceServer,
};
use scylla_proto::capabilities::v1::{GetCapabilitiesRequest, GetCapabilitiesResponse};
use scylla_testkit::TestApp;
use sqlx::PgPool;
use tonic::{Code, Request, Response, Status};

use crate::assert_code;

authz! {
    /// The route of the test service, on the caller.
    extend Actor as TestRoutes {
        /// The group of the test routes.
        permission everything = nil;
        /// The route `GetCapabilities` of the test service.
        permission get_capabilities = everything;
    }
}

/// What the test route does after it verifies the token.
#[derive(Clone, Copy)]
enum Mode {
    /// No check.
    Nothing,
    /// A check of a group permission, in place of the route permission.
    Group,
    /// A check of the route permission.
    Own,
    /// An error before any check.
    Fail,
}

/// A `CapabilityService` whose route `GetCapabilities` does what its mode tells.
struct TestRoute {
    /// The gate of the server.
    gate: Arc<dyn Gate>,
    /// What the route does.
    mode: Mode,
}

#[tonic::async_trait]
impl CapabilityService for TestRoute {
    async fn get_capabilities(
        &self,
        request: Request<GetCapabilitiesRequest>,
    ) -> Result<Response<GetCapabilitiesResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let me = caller.actor.id;
        let check = match self.mode {
            Mode::Nothing => None,
            Mode::Group => Some(TestRoutes::EVERYTHING.on(me)),
            Mode::Own => Some(TestRoutes::GET_CAPABILITIES.on(me)),
            Mode::Fail => return Err(Status::invalid_argument("refused by the test route")),
        };
        if let Some(check) = check {
            self.gate
                .authorize(&caller, check)
                .await
                .map_err(Error::from)?;
        }
        Ok(Response::new(GetCapabilitiesResponse::default()))
    }
}

/// Starts an edition with only the actors and the test route, and calls the route as a user.
async fn call(pool: PgPool, mode: Mode) -> Result<Response<GetCapabilitiesResponse>, Status> {
    let edition = Edition::builder("test")
        .migrations(&MIGRATOR)
        .authz::<Actor>()
        .authz::<TestRoutes>()
        .service(move |routes, ctx| {
            let route = TestRoute {
                gate: ctx.gate(),
                mode,
            };
            routes.add_service(CapabilityServiceServer::new(route));
        })
        .build();
    let app = TestApp::spawn(pool, edition).await;
    let user = app.user().await;
    let request = app.authed(user, GetCapabilitiesRequest {}).await;
    CapabilityServiceClient::new(app.channel.clone())
        .get_capabilities(request)
        .await
}

/// A route that verifies a token and checks no permission is refused.
#[sqlx::test(migrations = false)]
async fn a_route_with_no_check_is_refused(pool: PgPool) {
    // given
    let mode = Mode::Nothing;

    // when
    let result = call(pool, mode).await;

    // then
    assert_code(result, Code::PermissionDenied);
}

/// A route that checks a group permission in place of its own permission is refused.
#[sqlx::test(migrations = false)]
async fn a_route_that_checks_a_group_is_refused(pool: PgPool) {
    // given
    let mode = Mode::Group;

    // when
    let result = call(pool, mode).await;

    // then
    assert_code(result, Code::PermissionDenied);
}

/// A route that checks its own permission passes.
#[sqlx::test(migrations = false)]
async fn a_route_that_checks_its_permission_passes(pool: PgPool) {
    // given
    let mode = Mode::Own;

    // when
    let result = call(pool, mode).await;

    // then
    assert!(result.is_ok(), "{result:?}");
}

/// A route that fails before its check keeps its error.
#[sqlx::test(migrations = false)]
async fn a_route_that_fails_keeps_its_error(pool: PgPool) {
    // given
    let mode = Mode::Fail;

    // when
    let result = call(pool, mode).await;

    // then
    assert_code(result, Code::InvalidArgument);
}
