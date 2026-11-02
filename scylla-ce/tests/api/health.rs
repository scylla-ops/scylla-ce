//! The tests of the standard gRPC health service.

use sqlx::PgPool;
use tonic_health::pb::health_check_response::ServingStatus;

use crate::app;

/// A running server reports `SERVING`, for the probes of Kubernetes.
#[sqlx::test(migrations = false)]
async fn health_check_reports_serving(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let status = app.health_check().await.unwrap();

    // then
    assert_eq!(status, ServingStatus::Serving);
}
