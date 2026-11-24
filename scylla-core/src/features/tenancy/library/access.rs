//! The check of a route permission on one fragment: its author, or a role of its workspace.

use scylla_domain::authz::{Object, Permission};
use scylla_domain::identity::actors::Actor;
use scylla_domain::tenancy::library::{Fragment, FragmentCaller, FragmentHolder, FragmentId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{AccessError, Gate, Principal};

use crate::Error;

/// The permissions of a route on one fragment.
#[derive(Clone, Copy)]
pub(super) struct Route {
    /// The permission on the author itself, or `None` when the author has no route of its own.
    author: Option<Permission<Actor>>,
    /// The permission on the workspace of the fragment.
    holder: Permission<Workspace>,
}

/// The route `GetFragment`.
pub(super) const GET: Route = Route {
    author: None,
    holder: FragmentHolder::GET_FRAGMENT,
};

/// The route `SetFragmentFavorite`.
pub(super) const FAVORITE: Route = Route {
    author: None,
    holder: FragmentHolder::SET_FRAGMENT_FAVORITE,
};

/// The route `PublishFragmentVersion`.
pub(super) const PUBLISH: Route = Route {
    author: Some(FragmentCaller::PUBLISH_FRAGMENT_VERSION),
    holder: FragmentHolder::PUBLISH_FRAGMENT_VERSION,
};

/// The route `DeleteFragment`.
pub(super) const DELETE: Route = Route {
    author: Some(FragmentCaller::DELETE_FRAGMENT),
    holder: FragmentHolder::DELETE_FRAGMENT,
};

/// The route `AcceptOriginVersion`.
pub(super) const ACCEPT_ORIGIN: Route = Route {
    author: None,
    holder: FragmentHolder::ACCEPT_ORIGIN_VERSION,
};

/// The route `SkipOriginVersion`.
pub(super) const SKIP_ORIGIN: Route = Route {
    author: None,
    holder: FragmentHolder::SKIP_ORIGIN_VERSION,
};

/// The route `DismissOriginVersion`.
pub(super) const DISMISS_ORIGIN: Route = Route {
    author: None,
    holder: FragmentHolder::DISMISS_ORIGIN_VERSION,
};

/// The route `DetachOrigin`.
pub(super) const DETACH_ORIGIN: Route = Route {
    author: None,
    holder: FragmentHolder::DETACH_ORIGIN,
};

/// Checks a route on one fragment. Its author passes on itself while it reads the library of the
/// workspace. Another caller needs the route permission on the workspace. An unknown fragment and
/// a refusal get the same answer, which names the fragment, not its workspace.
pub(super) async fn authorize(
    gate: &dyn Gate,
    caller: &Principal,
    route: Route,
    id: FragmentId,
    found: Option<&Fragment>,
) -> Result<(), Error> {
    // Refuse an unknown fragment.
    let object = format!("fragment:{id}");
    let name = route.holder.name();
    let reason = format!("{name} on {object} is denied");
    let Some(fragment) = found else {
        return Err(AccessError::refused(caller, name, &object, reason).into());
    };

    // Check the author that still reads the library, else the workspace.
    let workspace = fragment.workspace_id;
    let check = match route.author {
        Some(author)
            if fragment.is_created_by(caller.actor.id)
                && reads(gate, caller, workspace).await? =>
        {
            author.on(caller.actor.id).within(workspace)
        }
        _ => route.holder.on(workspace),
    };

    // Hide the workspace in a refusal.
    match gate.authorize(caller, check).await {
        Ok(()) => Ok(()),
        Err(AccessError::Denied(_)) => Err(AccessError::Denied(reason).into()),
        Err(error) => Err(error.into()),
    }
}

/// Returns `true` if the caller reads the library of a workspace. It is not a route check.
async fn reads(gate: &dyn Gate, caller: &Principal, workspace: WorkspaceId) -> Result<bool, Error> {
    let names = [FragmentHolder::GET_FRAGMENT.name()];
    let answers = gate
        .permissions(caller, &Object::of(workspace), Some(workspace), &names)
        .await?;
    Ok(answers.first().copied().unwrap_or(false))
}
