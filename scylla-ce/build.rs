//! Builds the crate again when the build of the web app changes: it can add a file, and the embed
//! of the files does not see a new file.

use std::path::Path;

fn main() {
    let dist = Path::new("../web/dist");
    let watched = if dist.is_dir() {
        dist
    } else {
        Path::new("../web")
    };
    println!("cargo:rerun-if-changed={}", watched.display());
    if !dist.join("index.html").is_file() {
        println!("cargo:warning=no web app in web/dist: this binary serves the API only. Run `just web`, then build again.");
    }
}
