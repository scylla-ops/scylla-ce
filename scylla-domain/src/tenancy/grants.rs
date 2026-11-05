//! The access to an object: the owners of its workspace, the roles on it, and the permissions
//! of a caller.

use time::OffsetDateTime;

use crate::identity::actors::{Actor, ActorId, ActorKind};
use crate::projects::Project;
use crate::tenancy::catalog::Level;
use crate::tenancy::roles::RoleId;
use crate::tenancy::workspaces::Workspace;

/// The role of an owner of a workspace in a list of the access. No role of the model has this
/// name.
pub const OWNER: &str = "owner";

/// The kind of a membership, for the hooks: a user gets a role in a workspace or on one of its
/// objects, accepts an invitation, or becomes an owner.
pub const MEMBER_KIND: crate::ResourceKind = crate::ResourceKind::new("member");

/// The largest number of permissions in one `CheckPermissions`.
pub const MAX_CHECKS: usize = 50;

/// One access to an object: an owner of the workspace, or an actor with a role.
#[derive(Clone, Debug)]
pub struct AccessEntry {
    /// The actor.
    pub actor_id: ActorId,
    /// The kind of the actor.
    pub actor_kind: ActorKind,
    /// The display name of a user. Empty for the other kinds.
    pub display_name: String,
    /// The handle of a user, if it has one.
    pub handle: Option<String>,
    /// The role, or `owner`.
    pub role: String,
}

/// Where a page of a list of the access starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccessCursor {
    /// The owners of the workspace after this user.
    Owners(ActorId),
    /// The roles after this cursor of the permission system, or from the first role.
    Roles(Option<String>),
    /// The roles that are rows after this grant, or from the first one.
    Rows(Option<(ActorId, RoleId)>),
}

impl AccessCursor {
    /// The prefix of a token of the owners.
    const OWNERS: &'static str = "owners.";
    /// The prefix of a token of the roles.
    const ROLES: &'static str = "roles.";
    /// The prefix of a token of the roles that are rows.
    const ROWS: &'static str = "rows.";

    /// Reads the token of a page. Returns `None` for a text that is not a token.
    pub fn parse(token: &str) -> Option<Self> {
        if let Some(id) = token.strip_prefix(Self::OWNERS) {
            return ActorId::parse(id).ok().map(Self::Owners);
        }
        if let Some(after) = token.strip_prefix(Self::ROWS) {
            if after.is_empty() {
                return Some(Self::Rows(None));
            }
            let (actor, role) = after.split_once('.')?;
            let after = (ActorId::parse(actor).ok()?, RoleId::parse(role).ok()?);
            return Some(Self::Rows(Some(after)));
        }
        let cursor = token.strip_prefix(Self::ROLES)?;
        Some(Self::Roles((!cursor.is_empty()).then(|| cursor.to_owned())))
    }

    /// Returns the token of a page.
    pub fn token(&self) -> String {
        match self {
            Self::Owners(after) => format!("{}{after}", Self::OWNERS),
            Self::Roles(cursor) => format!("{}{}", Self::ROLES, cursor.as_deref().unwrap_or("")),
            Self::Rows(None) => Self::ROWS.to_owned(),
            Self::Rows(Some((actor, role))) => format!("{}{actor}.{role}", Self::ROWS),
        }
    }
}

/// One page of a list of the access: the owners first, then the roles.
#[derive(Clone, Debug)]
pub struct AccessPage {
    /// The entries of the page.
    pub entries: Vec<AccessEntry>,
    /// Where the next page starts, if one exists.
    pub next: Option<AccessCursor>,
}

/// Where a role is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrantScope {
    /// The level of the scope.
    pub level: Level,
    /// The id of the scope, as text.
    pub id: String,
    /// The name of a workspace or of a project. Empty for the platform.
    pub name: String,
}

/// One role that an actor holds on a scope.
#[derive(Clone, Debug)]
pub struct HeldRole {
    /// The scope.
    pub scope: GrantScope,
    /// The actor.
    pub actor: ActorId,
    /// The role in `Grant` and `Revoke`.
    pub role: String,
    /// The key of the role, or `owner`.
    pub key: String,
    /// The name of the role.
    pub name: String,
    /// The time of the grant, if it is known.
    pub granted_at: Option<OffsetDateTime>,
    /// The actor that gave the role, if it is known.
    pub granted_by: Option<ActorId>,
}

/// A member of a workspace or of a project, with its roles.
#[derive(Clone, Debug)]
pub struct Member {
    /// The profile of the actor. The role of the entry is empty.
    pub profile: AccessEntry,
    /// The roles.
    pub roles: Vec<HeldRole>,
}

crate::authz! {
    /// The routes on the members of a workspace.
    extend Workspace as WorkspaceMembers {
        /// The route `ListMembers` on a workspace.
        permission list_members = read;
        /// The route `GetMemberAccess` on a workspace.
        permission get_member_access = read;
    }
}

crate::authz! {
    /// The routes on the members of a project.
    extend Project as ProjectMembers {
        /// The route `ListMembers` on a project.
        permission list_members = view;
        /// The route `GetMemberAccess` on a project.
        permission get_member_access = view;
    }
}

crate::authz! {
    /// The routes of `AccessService` on the caller itself.
    extend Actor as AccessCaller {
        /// The route `CheckPermissions`: each caller asks for its own permissions.
        permission check_permissions = nil;
        /// The route `ListHeldPermissions`: each caller asks for its own permissions.
        permission list_held_permissions = nil;
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests of the tokens of a list of the access.

    use super::AccessCursor;
    use crate::identity::actors::ActorId;

    /// A token gives back its cursor, and a text that is not a token gives `None`.
    #[test]
    fn a_token_gives_back_its_cursor() {
        // given
        let cursors = [
            AccessCursor::Owners(ActorId::generate()),
            AccessCursor::Roles(None),
            AccessCursor::Roles(Some("abc.def".to_owned())),
            AccessCursor::Rows(None),
            AccessCursor::Rows(Some((
                ActorId::generate(),
                crate::tenancy::roles::RoleId::generate(),
            ))),
        ];

        // when
        let read: Vec<_> = cursors
            .iter()
            .map(|cursor| AccessCursor::parse(&cursor.token()))
            .collect();
        let bad = ["", "owners.nope", "other.x", "roles"].map(AccessCursor::parse);

        // then
        let expected: Vec<_> = cursors.into_iter().map(Some).collect();
        assert_eq!(read, expected);
        assert!(bad.iter().all(Option::is_none));
    }
}
