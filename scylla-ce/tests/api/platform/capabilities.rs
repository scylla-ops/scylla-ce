//! The tests of `CapabilityService` in the Community edition.

use sqlx::PgPool;

use crate::app;

/// The Community edition reports its name and no optional feature.
#[sqlx::test(migrations = false)]
async fn get_capabilities_reports_the_community_edition(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let response = app.get_capabilities().await.unwrap();

    // then
    assert_eq!(response.edition, "community");
    assert!(response.capabilities.is_empty());
}
