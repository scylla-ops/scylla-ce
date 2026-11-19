//! The tests of the gRPC reflection v1 and v1alpha: the services of the core and of the edition.

use prost::Message;
use prost_types::{FileDescriptorProto, FileDescriptorSet, ServiceDescriptorProto};
use scylla_testkit::TestApp;
use sqlx::PgPool;

/// A service of the core.
const CORE_SERVICE: &str = "scylla.projects.v1.ProjectService";

/// The service of the descriptor set of the test edition.
const EDITION_SERVICE: &str = "scylla.test.v1.EditionService";

/// Makes the encoded descriptor set of the test edition: one file with `EDITION_SERVICE`.
fn edition_set() -> &'static [u8] {
    let service = ServiceDescriptorProto {
        name: Some("EditionService".to_owned()),
        ..Default::default()
    };
    let file = FileDescriptorProto {
        name: Some("scylla/test/v1/edition_service.proto".to_owned()),
        package: Some("scylla.test.v1".to_owned()),
        service: vec![service],
        syntax: Some("proto3".to_owned()),
        ..Default::default()
    };
    let set = FileDescriptorSet { file: vec![file] };
    Vec::leak(set.encode_to_vec())
}

/// The reflection v1 and v1alpha list the services of the core and of the sets of the edition,
/// with no token.
#[sqlx::test(migrations = false)]
async fn server_reflection_info_lists_the_core_and_the_edition(pool: PgPool) {
    // given
    let edition = scylla_core::edition("test")
        .descriptors(edition_set())
        .build();
    let app = TestApp::spawn(pool, edition).await;

    // when
    let v1 = app.reflection_services().await.unwrap();
    let v1alpha = app.reflection_services_v1alpha().await.unwrap();

    // then
    for services in [v1, v1alpha] {
        assert!(
            services.iter().any(|name| name == CORE_SERVICE),
            "{services:?}"
        );
        assert!(
            services.iter().any(|name| name == EDITION_SERVICE),
            "{services:?}"
        );
    }
}
