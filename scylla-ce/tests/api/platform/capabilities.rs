//! The tests of `CapabilityService` in the Community edition.

use scylla_core::api::ServeOptions;
use scylla_testkit::TestApp;
use sqlx::PgPool;

use crate::app;

/// The Community edition reports its name, no optional feature, its version and the image of the
/// agent of this version.
#[sqlx::test(migrations = false)]
async fn get_capabilities_reports_the_community_edition(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let response = app.get_capabilities().await.unwrap();

    // then
    assert_eq!(response.edition, "community");
    assert!(response.capabilities.is_empty());
    assert_eq!(response.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        response.agent_image,
        format!(
            "ghcr.io/scylla-ops/scylla-agent:{}",
            env!("CARGO_PKG_VERSION")
        )
    );
}

/// The settings of the server replace the image of the agent.
#[sqlx::test(migrations = false)]
async fn get_capabilities_gives_the_image_of_the_settings(pool: PgPool) {
    // given
    let options = ServeOptions {
        agent_image: "registry.example.com/agent:7".to_owned(),
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;

    // when
    let response = app.get_capabilities().await.unwrap();

    // then
    assert_eq!(response.agent_image, "registry.example.com/agent:7");
}
