//! The start of each edition.

use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::admins::UserAdminHolder;
use scylla_domain::identity::users::AccountCaller;
use scylla_domain::platform::Platform;
use scylla_extension::{Edition, EditionBuilder};

use crate::features::identity::agents::Agents;
use crate::features::platform::capabilities::Capabilities;
use crate::features::projects::Projects;
use crate::features::tenancy::grants::Grants;
use crate::features::tenancy::workspaces::Workspaces;
use crate::infra::db::MIGRATOR;

/// Starts an edition with the core migrations, permission model and plugins, before those of
/// the edition.
pub fn edition(name: &'static str) -> EditionBuilder {
    Edition::builder(name)
        .migrations(&MIGRATOR)
        .authz::<Actor>()
        .authz::<AccountCaller>()
        .authz::<Platform>()
        .authz::<UserAdminHolder>()
        .plugin(Workspaces)
        .plugin(Agents)
        .plugin(Projects)
        .plugin(Capabilities)
        .plugin(Grants)
}
