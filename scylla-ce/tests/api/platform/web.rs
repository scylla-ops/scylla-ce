//! The tests of the files of the web app that the server serves on its address.

use std::fs;
use std::path::PathBuf;

use rust_embed::RustEmbed;
use scylla_core::api::ServeOptions;
use scylla_extension::Edition;
use scylla_proto::capabilities::v1::capability_service_client::CapabilityServiceClient;
use scylla_proto::capabilities::v1::GetCapabilitiesRequest;
use scylla_testkit::TestApp;
use sqlx::PgPool;

/// The files of a web app in a binary.
#[derive(RustEmbed)]
#[folder = "tests/api/platform/fixtures/web/"]
struct Built;

/// Makes an edition with a web app in its binary.
fn with_files() -> Edition {
    scylla_core::edition("community")
        .web_files(Built::get)
        .build()
}

/// Makes a folder of a built web app: `index.html` and one script.
fn web_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("scylla-web-{}", uuid_like()));
    fs::create_dir_all(dir.join("assets")).unwrap();
    fs::write(
        dir.join("index.html"),
        "<!doctype html><title>Scylla</title>",
    )
    .unwrap();
    fs::write(dir.join("assets/app.js"), "console.log('scylla');").unwrap();
    dir
}

/// Returns a text that is new for each call.
fn uuid_like() -> String {
    scylla_domain::identity::actors::ActorId::generate().to_string()
}

/// Reads a path of the server: its status, its content type and its body.
async fn get(app: &TestApp, path: &str) -> (u16, String, String) {
    let response = reqwest::get(format!("http://{}{path}", app.addr))
        .await
        .unwrap();
    let status = response.status().as_u16();
    let kind = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    (status, kind, response.text().await.unwrap())
}

/// A file gives the file, another path gives `index.html`, and the API still answers on the
/// same address.
#[sqlx::test(migrations = false)]
async fn the_server_serves_the_web_app(pool: PgPool) {
    // given
    let dir = web_dir();
    let options = ServeOptions {
        web_dir: Some(dir.clone()),
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;

    // when
    let home = get(&app, "/").await;
    let script = get(&app, "/assets/app.js").await;
    let route = get(&app, "/w/acme/projects").await;
    let api = CapabilityServiceClient::new(app.channel.clone())
        .get_capabilities(GetCapabilitiesRequest {})
        .await;

    // then
    assert_eq!(home.0, 200);
    assert!(home.2.contains("<title>Scylla</title>"));
    assert_eq!(script.0, 200);
    assert!(script.1.contains("javascript"), "{}", script.1);
    assert_eq!(route.2, home.2);
    assert!(api.is_ok());
    fs::remove_dir_all(dir).unwrap();
}

/// The web app of the binary is on the address of the API, with no setting.
#[sqlx::test(migrations = false)]
async fn the_server_serves_the_web_app_of_the_binary(pool: PgPool) {
    // given
    let app = TestApp::spawn_with(pool, with_files(), ServeOptions::default()).await;

    // when
    let home = get(&app, "/").await;
    let script = get(&app, "/assets/app-a1b2.js").await;
    let route = get(&app, "/w/acme/projects").await;
    let missing = get(&app, "/assets/missing.js").await;
    let api = CapabilityServiceClient::new(app.channel.clone())
        .get_capabilities(GetCapabilitiesRequest {})
        .await;

    // then
    assert_eq!(home.0, 200);
    assert!(home.2.contains("Scylla in the binary"));
    assert_eq!(script.0, 200);
    assert!(script.1.contains("javascript"), "{}", script.1);
    assert_eq!(route.2, home.2);
    assert_eq!(missing.2, home.2);
    assert!(api.is_ok());
}

/// A hashed file stays in the cache of the browser, and the browser asks again for a page.
#[sqlx::test(migrations = false)]
async fn the_web_app_of_the_binary_tells_its_cache(pool: PgPool) {
    // given
    let app = TestApp::spawn_with(pool, with_files(), ServeOptions::default()).await;
    let url = |path: &str| format!("http://{}{path}", app.addr);
    let client = reqwest::Client::new();

    // when
    let script = client.get(url("/assets/app-a1b2.js")).send().await.unwrap();
    let cache_of = |response: &reqwest::Response, name: &str| {
        response.headers()[name].to_str().unwrap().to_owned()
    };
    let etag = cache_of(&script, "etag");
    let again = client
        .get(url("/assets/app-a1b2.js"))
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    let home = client.get(url("/")).send().await.unwrap();
    let missing = client.get(url("/assets/missing.js")).send().await.unwrap();

    // then
    assert!(cache_of(&script, "cache-control").contains("immutable"));
    assert_eq!(again.status().as_u16(), 304);
    assert_eq!(cache_of(&home, "cache-control"), "no-cache");
    assert_eq!(cache_of(&missing, "cache-control"), "no-cache");
}

/// A browser gets the files compressed.
#[sqlx::test(migrations = false)]
async fn the_web_app_is_compressed(pool: PgPool) {
    // given
    let app = TestApp::spawn_with(pool, with_files(), ServeOptions::default()).await;

    // when
    let response = reqwest::Client::new()
        .get(format!("http://{}/assets/app-a1b2.js", app.addr))
        .header("accept-encoding", "gzip")
        .send()
        .await
        .unwrap();

    // then
    assert_eq!(response.headers()["content-encoding"], "gzip");
}

/// The folder of the settings comes before the files of the binary.
#[sqlx::test(migrations = false)]
async fn the_folder_of_the_settings_comes_first(pool: PgPool) {
    // given
    let dir = web_dir();
    let options = ServeOptions {
        web_dir: Some(dir.clone()),
        ..ServeOptions::default()
    };
    let app = TestApp::spawn_with(pool, with_files(), options).await;

    // when
    let home = get(&app, "/").await;

    // then
    assert!(home.2.contains("<title>Scylla</title>"));
    fs::remove_dir_all(dir).unwrap();
}

/// A gRPC call of an unknown service gets UNIMPLEMENTED in gRPC, and not a page.
#[sqlx::test(migrations = false)]
async fn an_unknown_grpc_call_gets_no_page(pool: PgPool) {
    // given
    let app = TestApp::spawn_with(pool, with_files(), ServeOptions::default()).await;
    let client = reqwest::Client::new();
    let url = format!("http://{}/unknown.v1.Service/Call", app.addr);

    for kind in ["application/grpc", "application/grpc-web+proto"] {
        // when
        let response = client
            .post(&url)
            .header("content-type", kind)
            .send()
            .await
            .unwrap();

        // then
        assert_eq!(response.status().as_u16(), 200);
        assert_eq!(response.headers()["grpc-status"], "12");
        assert_ne!(response.headers()["content-type"], "text/html");
    }

    // when
    let form = client.post(&url).send().await.unwrap();

    // then
    assert_eq!(form.status().as_u16(), 405);
}

/// A binary with no web app serves the API only.
#[sqlx::test(migrations = false)]
async fn a_binary_with_no_web_app_serves_the_api_only(pool: PgPool) {
    // given
    let edition = scylla_core::edition("community").build();
    let app = TestApp::spawn_with(pool, edition, ServeOptions::default()).await;

    // when
    let home = get(&app, "/").await;
    let api = CapabilityServiceClient::new(app.channel.clone())
        .get_capabilities(GetCapabilitiesRequest {})
        .await;

    // then
    assert!(!home.2.contains("<!doctype"));
    assert!(api.is_ok());
}
