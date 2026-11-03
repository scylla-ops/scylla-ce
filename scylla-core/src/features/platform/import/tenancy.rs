//! The import of the organizations: each one becomes a workspace, its admins its owners, and
//! its other members get the nearest fixed role.

use scylla_domain::authz::Relationship;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::slugs::Slug;
use scylla_domain::tenancy::workspaces::{NewSlug, NewWorkspace, Workspace, WorkspaceId};
use scylla_domain::Name;

use super::legacy::{Legacy, OldGrant};
use super::service::Run;
use crate::features::tenancy::workspaces;
use crate::Error;

/// The role of a user in a workspace, from a grant of the old product.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    /// An owner: an admin of the organization.
    Owner,
    /// An admin: a custom role of kind admin.
    Admin,
    /// A member.
    Member,
    /// A viewer.
    Viewer,
}

/// Returns the place of a grant of a user on an organization, or `None` for an App or an agent
/// role.
fn place(grant: &OldGrant) -> Option<Place> {
    if grant.principal_kind != "user" {
        return None;
    }
    match (grant.role_key.as_deref(), grant.role_kind.as_str()) {
        (Some("organization-admin"), _) => Some(Place::Owner),
        (Some("organization-member"), _) => Some(Place::Member),
        (Some("organization-viewer"), _) => Some(Place::Viewer),
        (Some(_), _) | (None, "agent") => None,
        (None, "admin") => Some(Place::Admin),
        (None, _) => Some(Place::Member),
    }
}

/// Imports each organization with a user that the import kept.
pub(super) async fn import(run: &mut Run<'_>, legacy: &Legacy) -> Result<(), Error> {
    for org in &legacy.organizations {
        // Find the users of the organization and their places, the owners first.
        let mut members: Vec<(ActorId, Place)> = Vec::new();
        for grant in legacy
            .grants
            .iter()
            .filter(|grant| grant.scope_kind == "organization" && grant.scope_id == org.id)
        {
            let (Some(user), Some(place)) = (run.users.get(&grant.principal_id), place(grant))
            else {
                continue;
            };
            if !members.iter().any(|(known, _)| known == user) {
                members.push((*user, place));
            }
        }
        members.sort_by_key(|(_, place)| *place != Place::Owner);

        // Refuse an organization with no user: a workspace needs an owner.
        let Some((creator, _)) = members.first().copied() else {
            run.report
                .skipped
                .push(format!("organization {}: no user to own it", org.name));
            continue;
        };

        // Create the workspace with its owners.
        let name = Name::parse(org.name.clone())
            .or_else(|_| Name::parse("Workspace".to_owned()))
            .expect("a valid name");
        let new = NewWorkspace {
            creator,
            slug: NewSlug::Derived(Slug::derive(name.as_str())),
            name,
        };
        let mut tx = run.pool.begin().await?;
        let workspace = workspaces::insert(&mut tx, &new).await?;
        for (user, place) in &members {
            if *place == Place::Owner {
                workspaces::add_owner(&mut tx, workspace.id, *user).await?;
            }
        }
        tx.commit().await?;

        // Keep the other places as roles, to write after the rows.
        for (user, place) in &members {
            if let Some(role) = role(workspace.id, *user, *place) {
                run.roles.push(role);
            }
        }
        run.workspaces
            .insert(org.id.clone(), (workspace.id, creator));
        run.report.workspaces += 1;
    }
    Ok(())
}

/// Returns the role of a place that is not an owner.
fn role(workspace: WorkspaceId, user: ActorId, place: Place) -> Option<Relationship> {
    match place {
        Place::Owner => None,
        Place::Admin => Some(Workspace::ADMIN.of(workspace, user)),
        Place::Member => Some(Workspace::MEMBER.of(workspace, user)),
        Place::Viewer => Some(Workspace::VIEWER.of(workspace, user)),
    }
}
