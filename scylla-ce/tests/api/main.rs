//! The gRPC API tests of the Community edition: one binary, one module per feature.

mod accounts;
mod agents;
mod authz;
mod browser;
mod capabilities;
mod grants;
mod grpc_web;
mod health;
mod idp;
mod invitations;
mod oidc;
mod owners;
mod ownership;
mod projects;
mod routes;
mod sessions;
mod sign_in;
mod tls;
mod tokens;
mod workspaces;

use scylla_testkit::TestApp;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

/// An id that no row has.
const UNKNOWN_ID: &str = "00000000-0000-0000-0000-000000000000";

/// Starts the Community edition on the test database.
async fn app(pool: PgPool) -> TestApp {
    TestApp::spawn(pool, scylla_ce::edition()).await
}

/// Checks that `status` is `INVALID_ARGUMENT` with one field violation on `field`.
fn assert_invalid_field(status: &Status, field: &str) {
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    let bad_request = status.get_details_bad_request().expect("BadRequest");
    let fields: Vec<_> = bad_request
        .field_violations
        .iter()
        .map(|violation| violation.field.as_str())
        .collect();
    assert_eq!(fields, vec![field]);
}

/// Checks the code of a refused call.
fn assert_code<T: std::fmt::Debug>(result: Result<T, Status>, code: Code) {
    let status = result.expect_err("the call must fail");
    assert_eq!(status.code(), code, "{status:?}");
}
