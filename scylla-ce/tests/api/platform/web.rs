//! The tests of the files of the web app that the server serves on its address.

use std::fs;
use std::path::PathBuf;

use scylla_core::api::ServeOptions;
use scylla_proto::capabilities::v1::capability_service_client::CapabilityServiceClient;
use scylla_proto::capabilities::v1::GetCapabilitiesRequest;
use scylla_testkit::TestApp;
use sqlx::PgPool;

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
