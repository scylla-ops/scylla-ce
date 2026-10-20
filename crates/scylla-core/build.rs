//! rust-embed rebuilds when an embedded file changes, not when Vite adds or removes a hashed chunk.
//! Must never fail: `dist` is gitignored, so a fresh checkout has no UI.

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    // Keep in sync with the `#[folder]` attribute in src/rest/ui/embedded.rs.
    let dist = manifest.join("../../web/dist");

    // A missing path counts as changed: the script re-runs when `pnpm build` creates it.
    println!("cargo:rerun-if-changed={}", dist.display());
    watch_subdirs(&dist);

    if !dist.join("index.html").is_file() {
        println!(
            "cargo:warning=web/dist/index.html not found; building a control plane \
             with no web UI. Run `just ui-build` and rebuild to embed it."
        );
    }
}

/// A directory mtime does not move when a nested file is added, so every subdirectory is watched.
fn watch_subdirs(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            let path = entry.path();
            println!("cargo:rerun-if-changed={}", path.display());
            watch_subdirs(&path);
        }
    }
}
