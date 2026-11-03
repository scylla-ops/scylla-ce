//! The access to an object: the owners of its workspace, the roles on it, and the permissions
//! of a caller.

use crate::identity::actors::{Actor, ActorId, ActorKind};

/// The role of an owner of a workspace in a list of the access. No role of the model has this
/// name.
pub const OWNER: &str = "owner";

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
}

impl AccessCursor {
    /// The prefix of a token of the owners.
    const OWNERS: &'static str = "owners.";
    /// The prefix of a token of the roles.
    const ROLES: &'static str = "roles.";

    /// Reads the token of a page. Returns `None` for a text that is not a token.
    pub fn parse(token: &str) -> Option<Self> {
        if let Some(id) = token.strip_prefix(Self::OWNERS) {
            return ActorId::parse(id).ok().map(Self::Owners);
        }
        let cursor = token.strip_prefix(Self::ROLES)?;
        Some(Self::Roles((!cursor.is_empty()).then(|| cursor.to_owned())))
    }

    /// Returns the token of a page.
    pub fn token(&self) -> String {
        match self {
            Self::Owners(after) => format!("{}{after}", Self::OWNERS),
            Self::Roles(cursor) => format!("{}{}", Self::ROLES, cursor.as_deref().unwrap_or("")),
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

crate::authz! {
    /// The routes of `AccessService` on the caller itself.
    extend Actor as AccessCaller {
        /// The route `CheckPermissions`: each caller asks for its own permissions.
        permission check_permissions = nil;
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
