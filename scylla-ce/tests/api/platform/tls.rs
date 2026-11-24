//! The test of the server with TLS.

use std::sync::Arc;

use scylla_core::api::{self, ServeOptions};
use scylla_testkit::test_access;
use sqlx::PgPool;
use tokio::net::TcpListener;
use tonic::transport::{Certificate, ClientTlsConfig, Endpoint, Identity};
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;
use tonic_health::pb::HealthCheckRequest;

/// A server with a certificate answers over TLS a client that trusts the certificate.
#[sqlx::test(migrations = false)]
async fn health_check_works_over_tls(pool: PgPool) {
    // given
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let (cert_pem, key_pem) = (cert.cert.pem(), cert.signing_key.serialize_pem());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let options = ServeOptions {
        tls: Some(Identity::from_pem(&cert_pem, &key_pem)),
        ..ServeOptions::default()
    };
    let edition = scylla_ce::edition();
    let access = Arc::new(test_access(pool.clone(), edition.authz_model().clone()));
    tokio::spawn(api::serve(
        listener,
        pool,
        edition,
        access,
        options,
        std::future::pending(),
    ));
    let tls = ClientTlsConfig::new()
        .ca_certificate(Certificate::from_pem(&cert_pem))
        .domain_name("localhost");
    let channel = Endpoint::from_shared(format!("https://{addr}"))
        .unwrap()
        .tls_config(tls)
        .unwrap()
        .connect()
        .await
        .unwrap();

    // when
    let request = HealthCheckRequest {
        service: String::new(),
    };
    let response = HealthClient::new(channel).check(request).await.unwrap();

    // then
    assert_eq!(response.into_inner().status(), ServingStatus::Serving);
}
