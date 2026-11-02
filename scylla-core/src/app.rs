//! The start of each edition.

use scylla_domain::actors::Actor;
use scylla_domain::users::AccountCaller;
use scylla_extension::{Edition, EditionBuilder};

use crate::db::MIGRATOR;
use crate::features::agents::Agents;
use crate::features::capabilities::Capabilities;
use crate::features::grants::Grants;
use crate::features::projects::Projects;
use crate::features::workspaces::Workspaces;

/// Starts an edition with the core migrations, permission model and plugins, before those of
/// the edition.
pub fn edition(name: &'static str) -> EditionBuilder {
    Edition::builder(name)
        .migrations(&MIGRATOR)
        .authz::<Actor>()
        .authz::<AccountCaller>()
        .plugin(Workspaces)
        .plugin(Agents)
        .plugin(Projects)
        .plugin(Capabilities)
        .plugin(Grants)
}
