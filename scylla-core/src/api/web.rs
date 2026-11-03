//! The files of the built web app: a path that is a file gives the file, and another path gives
//! `index.html`, so the routes of the web app load in the browser.

use std::path::Path;

use axum::routing::{get_service, MethodRouter};
use tower_http::services::{ServeDir, ServeFile};

/// Makes the service of the files of a folder, for GET and HEAD.
pub(super) fn files(dir: &Path) -> MethodRouter {
    let index = ServeFile::new(dir.join("index.html"));
    get_service(ServeDir::new(dir).fallback(index))
}
