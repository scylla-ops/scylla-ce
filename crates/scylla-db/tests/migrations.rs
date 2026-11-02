use std::fs;
use std::path::{Path, PathBuf};

fn migration_files(dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            migration_files(&path, files);
        } else if dir.ends_with("migrations") {
            files.push(path);
        }
    }
}

#[test]
fn every_migration_has_lf_line_endings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    for dir in ["migrations", "crates", "binaries"] {
        migration_files(&root.join(dir), &mut files);
    }
    assert!(!files.is_empty(), "no migration under {}", root.display());
    let crlf: Vec<&Path> = files
        .iter()
        .filter(|f| fs::read(f).unwrap().contains(&b'\r'))
        .map(|f| f.strip_prefix(&root).unwrap_or(f))
        .collect();
    assert!(
        crlf.is_empty(),
        "these migrations have a CR: {crlf:#?}. sqlx embeds the bytes and keeps their checksum, \
         so a server with this build does not start on a database that has the LF version. \
         Check out with LF (ARCHITECTURE.md)."
    );
}
