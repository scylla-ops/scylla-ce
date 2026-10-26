#![allow(clippy::unused_async_trait_impl)]

mod access;
mod agents;
mod apps;
mod auth;
mod bootstrap;
mod grants;
pub mod grpc;
mod host;
mod jobs;
mod organizations;
mod pipelines;
mod ports;
mod projects;
mod roles;
mod secrets;
mod triggers;
mod users;
mod workers;

pub use access::*;
pub use agents::*;
pub use apps::*;
pub use auth::*;
pub use bootstrap::*;
pub use grants::*;
pub use host::*;
pub use jobs::*;
pub use organizations::*;
pub use pipelines::*;
pub use ports::*;
pub use projects::*;
pub use roles::*;
pub use secrets::*;
pub use triggers::*;
pub use users::*;
pub use workers::*;

#[strata::edition(modules(
    Access,
    Projects,
    Secrets,
    Pipelines,
    Apps,
    Roles,
    Grants,
    Organizations,
    Users,
    Auth,
    Triggers,
    JobControl,
    Agents
))]
pub struct ScyllaCe;

#[strata::edition(extends = ScyllaCe, overrides(Access, Projects))]
pub struct ScyllaCeOnDb;

#[strata::overrides]
impl Access for ScyllaCeOnDb {
    type Authenticator = ScyllaSessions;
    type Permissions = CedarPermissions;
    type Forwarding = TrustedForwarding;
}

#[strata::overrides]
impl Projects for ScyllaCeOnDb {
    type Store = PgProjects;
    type Scopes = CedarScopes;
    type Directory = PgDirectory;
    type Jobs = DispatchJobs;
}
