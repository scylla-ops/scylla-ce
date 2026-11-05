//! The built-in roles: they come from the server and do not change.

use super::{Holder, RoleId};
use crate::authz::Resource;
use crate::identity::apps::AppHolder;
use crate::projects::pipelines::{PipelineAccess, PipelineHolder};
use crate::projects::Project;
use crate::tenancy::catalog::Level;
use crate::tenancy::workspaces::Workspace;
use crate::Id;

/// Where a built-in role gets its permissions and its holders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinSource {
    /// The rows of `instance_admins`. It has each permission.
    InstanceAdmins,
    /// A row of `roles` with its grants.
    Row(RoleId),
    /// A relation of the SpiceDB schema: the definition and the relation.
    Relation(&'static str, &'static str),
}

/// A built-in role.
#[derive(Clone, Copy, Debug)]
pub struct BuiltinRole {
    /// The key.
    pub key: &'static str,
    /// The name.
    pub name: &'static str,
    /// What the role does.
    pub description: &'static str,
    /// The level.
    pub level: Level,
    /// Who can hold the role.
    pub holder: Holder,
    /// Where it gets its permissions and its holders.
    pub source: BuiltinSource,
}

impl BuiltinRole {
    /// Returns the name of the role in `Grant`, `Revoke` and `ListAccess`: the relation of a
    /// role of the schema, else its key.
    pub const fn grant_name(&self) -> &'static str {
        match self.source {
            BuiltinSource::Relation(_, relation) => relation,
            _ => self.key,
        }
    }
}

/// The key of the role of the instance admins.
pub const INSTANCE_ADMIN: &str = "instance-admin";

/// The key of the role that creates workspaces.
pub const WORKSPACE_CREATOR: &str = "workspace-creator";

/// The row of the role that creates workspaces. The migration of the roles adds it.
pub const WORKSPACE_CREATOR_ROW: RoleId = Id::from_bits(1);

/// The built-in roles, in the order of the API.
pub const BUILTIN_ROLES: [BuiltinRole; 11] = [
    BuiltinRole {
        key: INSTANCE_ADMIN,
        name: "Instance admin",
        description: "Full control of the server, of every workspace and of every project.",
        level: Level::Platform,
        holder: Holder::People,
        source: BuiltinSource::InstanceAdmins,
    },
    BuiltinRole {
        key: WORKSPACE_CREATOR,
        name: "Workspace creator",
        description: "Creates workspaces. Becomes owner of each workspace that it creates.",
        level: Level::Platform,
        holder: Holder::People,
        source: BuiltinSource::Row(WORKSPACE_CREATOR_ROW),
    },
    BuiltinRole {
        key: "workspace-admin",
        name: "Workspace admin",
        description: "Manages one workspace and its projects.",
        level: Level::Workspace,
        holder: Holder::People,
        source: BuiltinSource::Relation(Workspace::NAME, Workspace::ADMIN.name()),
    },
    BuiltinRole {
        key: "workspace-member",
        name: "Workspace member",
        description: "Works in every project of the workspace. Cannot manage members or roles.",
        level: Level::Workspace,
        holder: Holder::People,
        source: BuiltinSource::Relation(Workspace::NAME, Workspace::MEMBER.name()),
    },
    BuiltinRole {
        key: "workspace-viewer",
        name: "Workspace viewer",
        description: "Sees the workspace, its projects, runs and logs. Changes nothing.",
        level: Level::Workspace,
        holder: Holder::People,
        source: BuiltinSource::Relation(Workspace::NAME, Workspace::VIEWER.name()),
    },
    BuiltinRole {
        key: "app-creator",
        name: "App creator",
        description: "Creates apps and agents. Give it with another role.",
        level: Level::Workspace,
        holder: Holder::People,
        source: BuiltinSource::Relation(Workspace::NAME, AppHolder::APP_CREATOR.name()),
    },
    BuiltinRole {
        key: "project-admin",
        name: "Project admin",
        description: "Full control of one project.",
        level: Level::Project,
        holder: Holder::People,
        source: BuiltinSource::Relation(Project::NAME, Project::ADMIN.name()),
    },
    BuiltinRole {
        key: "project-editor",
        name: "Project editor",
        description: "Edits and runs pipelines of one project. Cannot manage its secrets.",
        level: Level::Project,
        holder: Holder::People,
        source: BuiltinSource::Relation(Project::NAME, Project::EDITOR.name()),
    },
    BuiltinRole {
        key: "project-viewer",
        name: "Project viewer",
        description: "Sees one project, its runs and logs.",
        level: Level::Project,
        holder: Holder::People,
        source: BuiltinSource::Relation(Project::NAME, Project::VIEWER.name()),
    },
    BuiltinRole {
        key: "agent",
        name: "Agent",
        description: "Takes the jobs of every project of the workspace.",
        level: Level::Workspace,
        holder: Holder::Apps,
        source: BuiltinSource::Relation(Workspace::NAME, PipelineHolder::RUNNER.name()),
    },
    BuiltinRole {
        key: "project-agent",
        name: "Project agent",
        description: "Takes the jobs of one project.",
        level: Level::Project,
        holder: Holder::Apps,
        source: BuiltinSource::Relation(Project::NAME, PipelineAccess::RUNNER.name()),
    },
];

/// Returns a built-in role by its key.
pub fn builtin(key: &str) -> Option<&'static BuiltinRole> {
    BUILTIN_ROLES.iter().find(|role| role.key == key)
}
