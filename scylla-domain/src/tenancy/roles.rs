//! The roles: named sets of permissions of the catalog on one level. A built-in role comes from
//! the server, and a custom role is a row with one owner: the instance or one workspace.

use crate::identity::actors::{Actor, ActorId};
use crate::platform::Platform;
use crate::tenancy::catalog::Level;
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::{Id, Name};

mod builtin;

pub use builtin::{
    builtin, BuiltinRole, BuiltinSource, BUILTIN_ROLES, INSTANCE_ADMIN, WORKSPACE_CREATOR,
    WORKSPACE_CREATOR_ROW,
};

/// The id of a role that is a row.
pub type RoleId = Id<Role>;

/// A role, as the API shows it.
#[derive(Clone, Debug)]
pub struct Role {
    /// A fixed text for a built-in role, and the id of a custom role.
    pub key: String,
    /// The name.
    pub name: String,
    /// What the role does.
    pub description: String,
    /// The level where the role is given.
    pub level: Level,
    /// Who can hold the role.
    pub holder: Holder,
    /// The name of the role in `Grant`, `Revoke` and `ListAccess`.
    pub grant_name: String,
    /// `true` if the role comes from the server.
    pub builtin: bool,
    /// `true` if the role has each permission of its level.
    pub full_control: bool,
    /// The ids of the permissions, in the order of the catalog.
    pub permissions: Vec<String>,
    /// The number of grants of the role, on all the objects.
    pub holder_count: u32,
    /// The workspace that owns a custom role.
    pub workspace: Option<WorkspaceId>,
}

/// Who can hold a role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holder {
    /// People.
    People,
    /// Apps: machine identities.
    Apps,
}

impl Holder {
    /// Returns the name of the holders in the API.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::People => "people",
            Self::Apps => "apps",
        }
    }

    /// Reads the name of the holders.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "people" => Some(Self::People),
            "apps" => Some(Self::Apps),
            _ => None,
        }
    }
}

/// The largest number of characters in the description of a role.
pub const MAX_DESCRIPTION: usize = 500;

/// A role to make.
#[derive(Clone, Debug)]
pub struct NewRole {
    /// The id.
    pub id: RoleId,
    /// The workspace that owns the role, or `None` for the instance.
    pub workspace: Option<WorkspaceId>,
    /// The name.
    pub name: Name,
    /// What the role does.
    pub description: String,
    /// The level.
    pub level: Level,
    /// Who can hold the role.
    pub holder: Holder,
    /// The ids of the permissions.
    pub permissions: Vec<&'static str>,
    /// The actor that makes the role.
    pub created_by: ActorId,
}

/// A role that is a row, as the store returns it.
#[derive(Clone, Debug)]
pub struct RoleRow {
    /// The id.
    pub id: RoleId,
    /// The workspace that owns the role, or `None` for the instance.
    pub workspace: Option<WorkspaceId>,
    /// The name.
    pub name: String,
    /// What the role does.
    pub description: String,
    /// The level.
    pub level: Level,
    /// Who can hold the role.
    pub holder: Holder,
    /// `true` if the role comes from the server.
    pub builtin: bool,
    /// The ids of the permissions.
    pub permissions: Vec<String>,
    /// The number of grants of the role.
    pub holder_count: u32,
}

crate::authz! {
    /// The routes on the roles of the instance, and on the roles that the instance gives. An
    /// instance admin passes them, and a role of the catalog gives them.
    extend Platform as RoleAdmin {
        /// The route `ListRoles` with no workspace.
        permission list_roles = nil;
        /// The route `GetRole` with no workspace.
        permission get_role = nil;
        /// The route `CreateRole` for the instance.
        permission create_role = nil;
        /// The route `UpdateRole` of a role of the instance.
        permission update_role = nil;
        /// The route `DeleteRole` of a role of the instance.
        permission delete_role = nil;
        /// The route `Grant` on the platform.
        permission grant = nil;
        /// The route `Revoke` on the platform.
        permission revoke = nil;
    }
}

crate::authz! {
    /// The routes on the roles of a workspace.
    extend Workspace as RoleHolder {
        /// The route `ListRoles` with a workspace.
        permission list_roles = manage;
        /// The route `GetRole` with a workspace.
        permission get_role = manage;
        /// The route `CreateRole` for a workspace.
        permission create_role = manage;
        /// The route `UpdateRole` of a role of a workspace.
        permission update_role = manage;
        /// The route `DeleteRole` of a role of a workspace.
        permission delete_role = manage;
    }
}

crate::authz! {
    /// The routes of `RoleService` on the caller itself.
    extend Actor as RoleCaller {
        /// The route `ListPermissionCatalog`: each caller reads the catalog.
        permission list_permission_catalog = nil;
    }
}
