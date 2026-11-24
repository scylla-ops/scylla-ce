//! The edition of the `scylla-ce` binary.

use rust_embed::RustEmbed;
use scylla_extension::Edition;

/// The built web app. A build with no `web/dist` has no files, and serves the API only.
#[derive(RustEmbed)]
#[folder = "../web/dist/"]
#[allow_missing = true]
struct WebApp;

/// Makes the Community edition: the core plugins only.
pub fn edition() -> Edition {
    scylla_core::edition("community")
        .web_files(WebApp::get)
        .build()
}
