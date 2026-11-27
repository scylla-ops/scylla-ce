//! The lists of workspaces: the console of the workspaces of the instance, its search text and its
//! order, and the details that tell a workspace apart from the others in a list.

#[cfg(test)]
mod tests;

use time::OffsetDateTime;

use std::collections::HashSet;

use super::{Workspace, WorkspaceId};
use crate::identity::actors::ActorId;
use crate::platform::images::Avatar;
use crate::tenancy::grants::OWNER;
use crate::{DomainError, Page, PageRequest};

/// The search text of the console of the workspaces: trimmed, at most `MAX_LEN` characters, kept in
/// lower case. An empty text keeps each workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceSearch(String);

impl WorkspaceSearch {
    /// The maximum number of characters, after trim.
    pub const MAX_LEN: usize = 100;

    /// Trims the text and checks its length.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let trimmed = raw.trim();
        if trimmed.chars().count() > Self::MAX_LEN {
            return Err(DomainError::TooLong { max: Self::MAX_LEN });
        }
        Ok(Self(trimmed.chars().flat_map(char::to_lowercase).collect()))
    }

    /// Returns the text, in lower case.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The order of the console of the workspaces. Two workspaces with the same keys go in id order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceOrder {
    /// The name, with no difference between upper and lower case, then the name of the first
    /// owner.
    #[default]
    Name,
    /// The newest workspace first.
    Newest,
    /// The oldest workspace first.
    Oldest,
    /// The workspace with the newest activity first. No activity comes last.
    LastActivity,
}

impl WorkspaceOrder {
    /// Returns the key of the order that the SQL of the list reads.
    pub fn key(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Newest => "newest",
            Self::Oldest => "oldest",
            Self::LastActivity => "activity",
        }
    }
}

/// The input of the console of the workspaces.
#[derive(Clone, Debug, Default)]
pub struct ListAllWorkspaces {
    /// Only the workspaces that have this text.
    pub search: WorkspaceSearch,
    /// The order of the workspaces.
    pub order: WorkspaceOrder,
    /// `true` keeps only the workspaces where the caller is not an owner and has no role.
    pub others_only: bool,
    /// The page to return.
    pub page: PageRequest<WorkspaceId>,
}

/// One page of the console of the workspaces, with the number of workspaces on all the pages.
#[derive(Clone, Debug)]
pub struct AllWorkspacesPage {
    /// The workspaces of the page.
    pub page: Page<Workspace, WorkspaceId>,
    /// The number of workspaces that the search keeps.
    pub total: u32,
}

/// The owner of a workspace, as a list of workspaces shows it.
#[derive(Clone, Debug)]
pub struct WorkspaceOwner {
    /// The user.
    pub user: ActorId,
    /// The display name.
    pub display_name: String,
    /// The handle, if the user has one.
    pub handle: Option<String>,
    /// The primary email, only for a caller that sees each workspace of the instance.
    pub email: Option<String>,
    /// The avatar that the user chose.
    pub avatar: Option<Avatar>,
}

/// The details of a workspace that tell it apart from the others in a list.
#[derive(Clone, Debug)]
pub struct WorkspaceDetails {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The oldest owner, if the workspace has one.
    pub owner: Option<WorkspaceOwner>,
    /// The number of actors with a role on the workspace.
    pub member_count: u32,
    /// The number of projects.
    pub project_count: u32,
    /// The time of the newest entry of the activity feed of the workspace.
    pub last_activity_at: Option<OffsetDateTime>,
    /// The role of the caller in the workspace, if it has one.
    pub role: Option<String>,
}

/// The roles on one workspace: the owners, the roles of the schema and the roles that are rows.
#[derive(Clone, Debug, Default)]
pub struct WorkspaceRoles {
    /// The owners.
    pub owners: Vec<ActorId>,
    /// Each role of the schema: the actor and the name of the relation.
    pub relations: Vec<(ActorId, String)>,
    /// Each role that is a row: the actor and the name of the role.
    pub rows: Vec<(ActorId, String)>,
}

/// The roles of the schema, from the strongest.
const RELATIONS: [&str; 3] = [
    Workspace::ADMIN.name(),
    Workspace::MEMBER.name(),
    Workspace::VIEWER.name(),
];

impl WorkspaceRoles {
    /// Returns the number of actors with a role: each actor counts one time.
    pub fn member_count(&self) -> u32 {
        let actors: HashSet<ActorId> = self
            .owners
            .iter()
            .copied()
            .chain(self.relations.iter().map(|(actor, _)| *actor))
            .chain(self.rows.iter().map(|(actor, _)| *actor))
            .collect();
        u32::try_from(actors.len()).unwrap_or(u32::MAX)
    }

    /// Returns the strongest role of an actor: owner, then the roles of the schema, then the first
    /// role that is a row. `None` if the actor has no role.
    pub fn role_of(&self, actor: ActorId) -> Option<String> {
        if self.owners.contains(&actor) {
            return Some(OWNER.to_owned());
        }
        let held = |name: &str| {
            self.relations
                .iter()
                .any(|(holder, role)| *holder == actor && role == name)
        };
        if let Some(relation) = RELATIONS.into_iter().find(|name| held(name)) {
            return Some(relation.to_owned());
        }
        let other = self.relations.iter().chain(&self.rows);
        other
            .filter(|(holder, _)| *holder == actor)
            .map(|(_, role)| role.clone())
            .next()
    }
}
