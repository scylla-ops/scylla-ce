//! Where the resources live: the workspaces, the invitations, the roles that `AccessService`
//! gives, the saved layouts of the dashboard of a workspace, and its library.

pub(crate) mod dashboards;
pub(crate) mod grants;
pub(crate) mod invitations;
pub mod library;
pub(crate) mod roles;
pub mod workspaces;
