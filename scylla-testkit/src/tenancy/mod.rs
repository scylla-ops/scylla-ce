//! The `TestApp` helpers of the tenancy RPCs: the workspaces, the roles and
//! the permissions of `AccessService`, the invitations, the saved layouts of the dashboard, and
//! the library.

mod access;
mod dashboards;
mod invitations;
mod library;
mod roles;
mod workspaces;

pub use access::Role;
pub use dashboards::dashboard_widgets;
pub use library::{env_node, fragment_link, fragment_request, literal_parameter};
pub use roles::RoleInput;
