//! The `TestApp` helpers of the tenancy RPCs: the workspaces, the roles and
//! the permissions of `AccessService`, the invitations, and the saved layouts of the dashboard.

mod access;
mod dashboards;
mod invitations;
mod roles;
mod workspaces;

pub use access::Role;
pub use dashboards::dashboard_widgets;
pub use roles::RoleInput;
