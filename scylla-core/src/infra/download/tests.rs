//! Unit tests of a download: the public addresses, the checks of the URL, and the answers of a
//! server that the download refuses.

use std::net::IpAddr;

use axum::body::{Body, Bytes};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;

use super::address::is_public;
use super::fetch::{download, Reach, Refusal};

/// The content types of the tests.
const PNG: [&str; 1] = ["image/png"];

/// The limit of the tests, in bytes.
const MAX: usize = 16;

/// Starts a server with the answers of the tests, and returns its URL.
async fn server() -> String {
    let small = || ([(header::CONTENT_TYPE, "image/png")], vec![7_u8; MAX]);
    let large = || ([(header::CONTENT_TYPE, "image/png")], vec![7_u8; MAX + 1]);
    let stream = || {
        let chunks = (0..4).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![7_u8; MAX / 2])));
        let body = Body::from_stream(tokio_stream::iter(chunks));
        ([(header::CONTENT_TYPE, "image/png")], body)
    };
    let redirect = || (StatusCode::FOUND, [(header::LOCATION, "/small")]).into_response();
    let html = || ([(header::CONTENT_TYPE, "text/html")], vec![7_u8; MAX]);
    let router = Router::new()
        .route("/small", get(move || async move { small() }))
        .route("/large", get(move || async move { large() }))
        .route("/stream", get(move || async move { stream() }))
        .route("/redirect", get(move || async move { redirect() }))
        .route("/html", get(move || async move { html() }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    url
}

/// The loopback, private, link-local, shared and reserved addresses are not public, also in an
/// IPv6 address that holds them.
#[test]
fn only_a_public_address_is_public() {
    // given
    let private = [
        "127.0.0.1",
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254",
        "100.64.0.1",
        "0.0.0.0",
        "255.255.255.255",
        "224.0.0.1",
        "::1",
        "::",
        "fe80::1",
        "fc00::1",
        "fd00:ec2::254",
        "::ffff:127.0.0.1",
        "::ffff:10.0.0.1",
        "64:ff9b::a00:1",
        "2002:7f00:1::",
        "2001:db8::1",
    ];
    let public = [
        "140.82.112.3",
        "8.8.8.8",
        "2606:4700::1111",
        "::ffff:8.8.8.8",
    ];

    // when
    let parse = |text: &str| text.parse::<IpAddr>().unwrap();

    // then
    for ip in private {
        assert!(!is_public(parse(ip)), "{ip}");
    }
    for ip in public {
        assert!(is_public(parse(ip)), "{ip}");
    }
}

/// A public download needs HTTPS, a URL with no credential, and a host with public addresses
/// only.
#[tokio::test]
async fn a_public_download_refuses_http_a_credential_and_a_private_host() {
    // given
    let cases = [
        ("not a URL", Refusal::NotAUrl),
        ("http://example.com/photo.png", Refusal::NotHttps),
        ("ftp://example.com/photo.png", Refusal::NotHttps),
        (
            "https://user:secret@example.com/photo.png",
            Refusal::Credential,
        ),
        ("https://127.0.0.1/photo.png", Refusal::NotPublic),
        ("https://[::1]/photo.png", Refusal::NotPublic),
        (
            "https://169.254.169.254/latest/meta-data",
            Refusal::NotPublic,
        ),
        ("https://localhost/photo.png", Refusal::NotPublic),
    ];

    for (url, refusal) in cases {
        // when
        let result = download(url, Reach::Public, &PNG, MAX).await;

        // then
        assert_eq!(result, Err(refusal), "{url}");
    }
}

/// A file of the content type and in the limit comes whole.
#[tokio::test]
async fn a_download_gives_the_file() {
    // given
    let url = server().await;

    // when
    let file = download(&format!("{url}/small"), Reach::Local, &PNG, MAX).await;

    // then
    assert_eq!(file, Ok(vec![7_u8; MAX]));
}

/// A redirect, another content type, a status that is not a success and a file above the limit,
/// with or without its size in the headers, are refused.
#[tokio::test]
async fn a_download_refuses_a_redirect_another_type_and_a_large_file() {
    // given
    let url = server().await;
    let cases = [
        ("/redirect", Refusal::Redirect),
        ("/html", Refusal::ContentType),
        ("/missing", Refusal::Status(404)),
        ("/large", Refusal::TooLarge(MAX)),
        ("/stream", Refusal::TooLarge(MAX)),
    ];

    for (path, refusal) in cases {
        // when
        let result = download(&format!("{url}{path}"), Reach::Local, &PNG, MAX).await;

        // then
        assert_eq!(result, Err(refusal), "{path}");
    }
}
