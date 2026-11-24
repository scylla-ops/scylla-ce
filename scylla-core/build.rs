//! Rebuilds the crate when a migration changes: `sqlx::migrate!` embeds the files.

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
