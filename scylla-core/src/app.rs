//! The start of each edition.

use scylla_extension::{Edition, EditionBuilder};

use crate::access::CORE_SCHEMA;
use crate::db::MIGRATOR;
use crate::features::agents::Agents;
use crate::features::capabilities::Capabilities;
use crate::features::projects::Projects;

/// Starts an edition with the core migrations, schema and plugins, before those of the edition.
pub fn edition(name: &'static str) -> EditionBuilder {
    Edition::builder(name)
        .migrations(&MIGRATOR)
        .authz_schema(CORE_SCHEMA)
        .plugin(Agents)
        .plugin(Projects)
        .plugin(Capabilities)
}
