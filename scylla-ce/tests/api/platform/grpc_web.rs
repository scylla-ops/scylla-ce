//! The tests of gRPC-Web: a browser frame over HTTP/1.1, and the CORS preflight.

use scylla_core::api::ServeOptions;
use scylla_testkit::TestApp;
use sqlx::PgPool;

/// The origin that the tests allow.
const ORIGIN: &str = "https://app.example";

/// Starts the Community edition that allows `ORIGIN`.
async fn app(pool: PgPool) -> TestApp {
    let options = ServeOptions {
        cors_origins: vec![ORIGIN.to_owned()],
        ..ServeOptions::default()
    };
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// A gRPC-Web call over HTTP/1.1 gets the answer and the gRPC status in the body.
#[sqlx::test(migrations = false)]
async fn health_check_works_over_grpc_web(pool: PgPool) {
    // given
    let app = app(pool).await;
    let url = format!("http://{}/grpc.health.v1.Health/Check", app.addr);
    let empty_message = vec![0u8, 0, 0, 0, 0];

    // when
    let response = reqwest::Client::new()
        .post(url)
        .header("content-type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .body(empty_message)
        .send()
        .await
        .unwrap();

    // then
    assert_eq!(response.status(), 200);
    assert_eq!(response.version(), reqwest::Version::HTTP_11);
    let body = response.bytes().await.unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("grpc-status:0"), "{body:?}");
}

/// A preflight from an allowed origin passes, and one from another origin gets no grant.
#[sqlx::test(migrations = false)]
async fn cors_allows_only_the_listed_origins(pool: PgPool) {
    // given
    let app = app(pool).await;
    let url = format!(
        "http://{}/scylla.projects.v1.ProjectService/ListProjects",
        app.addr
    );
    let preflight = |origin: &str| {
        reqwest::Client::new()
            .request(reqwest::Method::OPTIONS, &url)
            .header("origin", origin)
            .header("access-control-request-method", "POST")
            .header(
                "access-control-request-headers",
                "authorization,content-type,x-grpc-web",
            )
            .send()
    };

    // when
    let allowed = preflight(ORIGIN).await.unwrap();
    let other = preflight("https://evil.example").await.unwrap();

    // then
    let granted = allowed.headers().get("access-control-allow-origin");
    assert_eq!(granted.and_then(|value| value.to_str().ok()), Some(ORIGIN));
    assert!(other.headers().get("access-control-allow-origin").is_none());
}
