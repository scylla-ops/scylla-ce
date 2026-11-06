//! The kinds of notifications of the core, and the facts that the services record.

use std::sync::Arc;
use std::time::Duration;

use scylla_domain::authz::Resource;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::admins::UserAdminHolder;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::invitations::{self, Invitation};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{Access, Collapse, Event, NotificationKind, Params};

use super::audience::{Invitee, Person, PersonAndAdmins, PersonAndOwners};

/// The key `invitation.received`.
pub(crate) const INVITATION_RECEIVED: &str = "invitation.received";
/// The key `invitation.accepted`.
pub(crate) const INVITATION_ACCEPTED: &str = "invitation.accepted";
/// The key `invitation.declined`.
pub(crate) const INVITATION_DECLINED: &str = "invitation.declined";
/// The key `role.granted`.
pub(crate) const ROLE_GRANTED: &str = "role.granted";
/// The key `role.revoked`.
pub(crate) const ROLE_REVOKED: &str = "role.revoked";
/// The key `owner.added`.
pub(crate) const OWNER_ADDED: &str = "owner.added";
/// The key `owner.removed`.
pub(crate) const OWNER_REMOVED: &str = "owner.removed";
/// The key `admin.granted`.
pub(crate) const ADMIN_GRANTED: &str = "admin.granted";
/// The key `admin.revoked`.
pub(crate) const ADMIN_REVOKED: &str = "admin.revoked";
/// The key `instance_role.changed`.
pub(crate) const INSTANCE_ROLE_CHANGED: &str = "instance_role.changed";
/// The key `security.password_changed`.
pub(crate) const PASSWORD_CHANGED: &str = "security.password_changed";
/// The key `security.session_reuse`.
pub(crate) const SESSION_REUSE: &str = "security.session_reuse";
/// The key `security.account_locked`.
pub(crate) const ACCOUNT_LOCKED: &str = "security.account_locked";
/// The key `security.email_changed`.
pub(crate) const EMAIL_CHANGED: &str = "security.email_changed";
/// The key `security.identity_linked`.
pub(crate) const IDENTITY_LINKED: &str = "security.identity_linked";
/// The key `security.identity_unlinked`.
pub(crate) const IDENTITY_UNLINKED: &str = "security.identity_unlinked";
/// The key `token.created`.
pub(crate) const TOKEN_CREATED: &str = "token.created";

/// The category of the access to a workspace.
const ACCESS: &str = "access";
/// The category of the facts that a person must know.
const SECURITY: &str = "security";
/// The kind of the object of a fact about a person.
pub(super) const PERSON: &str = "user";
/// The parameter with the roles that a person got.
const ADDED: &str = "added";
/// The parameter with the roles that a person lost.
const REMOVED: &str = "removed";
/// The time in which the facts about the roles of a person are one row: a dialog sends one call for
/// each click.
const ROLE_WINDOW: Duration = Duration::from_secs(5 * 60);

/// Returns the kinds of the core.
pub(super) fn all() -> Vec<NotificationKind> {
    let roles = Collapse::Merge {
        window: ROLE_WINDOW,
        merge: merge_roles,
    };
    let person = || Arc::new(Person("subject"));
    let inviter = || Arc::new(Person("inviter"));
    let invitations = Access::Holds(Workspace::LIST_INVITATIONS.name());
    let workspace = Access::Holds(Workspace::GET_WORKSPACE.name());
    let instance = Access::Holds(UserAdminHolder::LIST_USERS.name());
    let new = NotificationKind::new;
    let mut kinds = vec![
        new(INVITATION_RECEIVED, ACCESS, Arc::new(Invitee))
            .collapse(Collapse::Replace)
            .needs_action()
            .mandatory(),
        new(INVITATION_ACCEPTED, ACCESS, inviter()).access(invitations),
        new(INVITATION_DECLINED, ACCESS, inviter()).access(invitations),
        new(ROLE_GRANTED, ACCESS, person())
            .collapse(roles)
            .mandatory(),
        new(ROLE_REVOKED, ACCESS, person())
            .collapse(roles)
            .mandatory(),
        new(OWNER_ADDED, ACCESS, Arc::new(PersonAndOwners))
            .access(workspace)
            .mandatory(),
        new(OWNER_REMOVED, ACCESS, Arc::new(PersonAndOwners))
            .access(workspace)
            .mandatory(),
        new(ADMIN_GRANTED, SECURITY, Arc::new(PersonAndAdmins))
            .access(instance)
            .mandatory(),
        new(ADMIN_REVOKED, SECURITY, Arc::new(PersonAndAdmins))
            .access(instance)
            .mandatory(),
        new(INSTANCE_ROLE_CHANGED, SECURITY, person())
            .collapse(roles)
            .mandatory(),
    ];
    for key in [
        PASSWORD_CHANGED,
        SESSION_REUSE,
        ACCOUNT_LOCKED,
        EMAIL_CHANGED,
        IDENTITY_LINKED,
        IDENTITY_UNLINKED,
        TOKEN_CREATED,
    ] {
        kinds.push(new(key, SECURITY, person()).mandatory());
    }
    kinds
}

/// Makes a fact about a person.
pub(crate) fn about(kind: &str, user: ActorId) -> Event {
    Event::new(kind).subject(PERSON, user)
}

/// Makes the fact of an invitation. A fact for a person that verified its email later has its own
/// dedupe key, and the group keeps one row.
pub(crate) fn invitation_received(invitation: &Invitation, later: Option<ActorId>) -> Event {
    let id = invitation.id;
    let dedupe = match later {
        Some(user) => format!("invitation:{id}:received:{user}"),
        None => format!("invitation:{id}:received"),
    };
    Event::new(INVITATION_RECEIVED)
        .subject(invitations::KIND.as_str(), id)
        .actor(invitation.invited_by)
        .workspace(invitation.workspace_id)
        .param("role", invitation.role.clone())
        .group(format!("invitation:{id}"))
        .dedupe(dedupe)
}

/// Makes a fact for the person that invited, about the answer of a person.
pub(crate) fn invitation_answered(kind: &str, invitation: &Invitation, by: ActorId) -> Event {
    about(kind, invitation.invited_by)
        .actor(by)
        .workspace(invitation.workspace_id)
        .param("role", invitation.role.clone())
}

/// Makes the fact of a role that a person got or lost on an object of a workspace. The name of a
/// custom role goes in the row, because the kinds of the web app know only the roles of the schema.
pub(crate) fn role_change(
    kind: &str,
    role: &str,
    (object_kind, object): (&str, &str),
    workspace: WorkspaceId,
    user: ActorId,
    by: ActorId,
) -> Event {
    let field = if kind == ROLE_GRANTED { ADDED } else { REMOVED };
    let event = about(kind, user)
        .actor(by)
        .workspace(workspace)
        .param(field, one_role(role))
        .group(format!("access:{object_kind}:{object}:{user}"));
    match ProjectId::parse(object) {
        Ok(project) if object_kind == Project::NAME => event.project(project),
        _ => event,
    }
}

/// Makes the fact of a role of the instance that a person got or lost.
pub(crate) fn instance_role_change(granted: bool, role: &str, user: ActorId, by: ActorId) -> Event {
    let field = if granted { ADDED } else { REMOVED };
    about(INSTANCE_ROLE_CHANGED, user)
        .actor(by)
        .param(field, one_role(role))
        .group(format!("access:platform:{user}"))
}

/// Returns a list with one role, as the text of a parameter.
fn one_role(role: &str) -> String {
    write_roles(&[role.to_owned()])
}

/// Reads a list of roles from a parameter.
fn roles_of(params: &Params, field: &str) -> Vec<String> {
    params
        .get(field)
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_default()
}

/// Writes a list of roles as the text of a parameter.
fn write_roles(roles: &[String]) -> String {
    serde_json::to_string(roles).unwrap_or_default()
}

/// Adds the roles of a new fact to a row. A role that comes after it went, or goes after it came,
/// takes both changes out.
fn merge_roles(row: &mut Params, new: &Params) {
    let mut lists = [roles_of(row, ADDED), roles_of(row, REMOVED)];
    for (index, field) in [ADDED, REMOVED].into_iter().enumerate() {
        for role in roles_of(new, field) {
            let other = &mut lists[1 - index];
            if let Some(place) = other.iter().position(|item| *item == role) {
                other.remove(place);
            } else if !lists[index].contains(&role) {
                lists[index].push(role);
            }
        }
    }
    row.insert(ADDED.to_owned(), write_roles(&lists[0]));
    row.insert(REMOVED.to_owned(), write_roles(&lists[1]));
    for (key, value) in new {
        if key != ADDED && key != REMOVED {
            row.insert(key.clone(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns parameters with one list of one role.
    fn params(field: &str, role: &str) -> Params {
        Params::from([(field.to_owned(), one_role(role))])
    }

    /// A second role joins the list, and the same role does not.
    #[test]
    fn merge_adds_each_role_one_time() {
        // given
        let mut row = params(ADDED, "viewer");

        // when
        merge_roles(&mut row, &params(ADDED, "member"));
        merge_roles(&mut row, &params(ADDED, "member"));

        // then
        assert_eq!(roles_of(&row, ADDED), ["viewer", "member"]);
        assert!(roles_of(&row, REMOVED).is_empty());
    }

    /// A role that goes after it came takes both changes out.
    #[test]
    fn merge_takes_out_a_change_that_goes_back() {
        // given
        let mut row = params(ADDED, "viewer");

        // when
        merge_roles(&mut row, &params(REMOVED, "viewer"));
        merge_roles(&mut row, &params(REMOVED, "member"));

        // then
        assert!(roles_of(&row, ADDED).is_empty());
        assert_eq!(roles_of(&row, REMOVED), ["member"]);
    }
}
