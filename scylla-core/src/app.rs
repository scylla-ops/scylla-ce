//! The start of each edition.

use scylla_extension::{Edition, EditionBuilder};

use crate::db::MIGRATOR;
use crate::features::api_keys::ApiKeys;
use crate::features::capabilities::Capabilities;
use crate::features::projects::Projects;

/// Starts an edition with the core migrations and the core plugins, before those of the edition.
pub fn edition(name: &'static str) -> EditionBuilder {
    Edition::builder(name)
        .migrations(&MIGRATOR)
        .plugin(ApiKeys)
        .plugin(Projects)
        .plugin(Capabilities)
}
