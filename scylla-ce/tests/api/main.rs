//! The gRPC API tests of the Community edition: one binary, one module per feature.

mod api_keys;
mod capabilities;
mod health;
mod projects;
mod tls;

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
