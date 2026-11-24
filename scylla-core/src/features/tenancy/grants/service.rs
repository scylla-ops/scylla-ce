//! The logic of `AccessService`: the actors that can get a role, and the pages of a list of the
//! access.

use scylla_domain::authz::Object;
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::apps;
use scylla_domain::tenancy::grants::{AccessCursor, AccessEntry, AccessPage, MEMBER_KIND};
use scylla_domain::tenancy::roles::RoleId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageSize;
use scylla_extension::{CreateContext, DeleteContext, Gate, Hooks, Rejection};
use sqlx::PgPool;
use uuid::Uuid;

use super::store;
use crate::features::identity::{actors, apps as app_feature};
use crate::Error;

/// Passes an actor that can get a role in a workspace: an actor that exists, and that is not an
/// App of another workspace. An unknown actor gives `NotFound("actor")`, and an App of another
/// workspace `ACTOR_OUTSIDE_WORKSPACE`.
pub(super) async fn check_actor(
    pool: &PgPool,
    actor: ActorId,
    workspace: WorkspaceId,
) -> Result<Actor, Error> {
    let actor = actors::find(pool, actor).await?;
    if actor.kind != apps::KIND
        || app_feature::workspace_of(pool, actor.id).await? == Some(workspace)
    {
        return Ok(actor);
    }
    let message = "an app gets roles only in its own workspace";
    Err(Error::Rejected(Rejection::new(
        "ACTOR_OUTSIDE_WORKSPACE",
        message,
    )))
}

/// Runs the `BeforeCreate` hooks of a user that gets a role in a workspace or on one of its
/// objects: `(user, object, role)`. A hook can refuse, for example over a limit of members.
pub(crate) async fn join(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    workspace: WorkspaceId,
    (user, object, role): (ActorId, String, &str),
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    let hook =
        CreateContext::new(MEMBER_KIND, actor, workspace).with_member(user, Some(object), role);
    hooks.before_create(&mut tx, &hook).await?;
    tx.commit().await?;
    Ok(())
}

/// Runs the `AfterDelete` hooks of an actor that loses a role: `(actor, object, role)`.
pub(crate) async fn leave(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    workspace: WorkspaceId,
    (member, object, role): (ActorId, String, &str),
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    let hook = DeleteContext::new(MEMBER_KIND, actor, workspace).with_member(member, object, role);
    hooks.after_delete(&mut tx, &hook).await?;
    tx.commit().await?;
    Ok(())
}

/// Returns one page of the access to an object: the owners of a workspace first, in user id
/// order, then the roles, in the order of SpiceDB. Returns `None` for a token that does not fit
/// the object, or that SpiceDB refuses.
pub(super) async fn list_access(
    pool: &PgPool,
    gate: &dyn Gate,
    object: &Object,
    owners_of: Option<WorkspaceId>,
    start: Option<AccessCursor>,
    size: PageSize,
) -> Result<Option<AccessPage>, Error> {
    // Find where the page starts: in the owners of a workspace, or in the roles.
    let (owners, cursor) = match (start, owners_of) {
        (None, Some(workspace)) => (Some((workspace, None)), None),
        (None, None) => (None, None),
        (Some(AccessCursor::Owners(after)), Some(workspace)) => {
            (Some((workspace, Some(after))), None)
        }
        (Some(AccessCursor::Owners(_)), None) => return Ok(None),
        (Some(AccessCursor::Roles(cursor)), _) => (None, cursor),
        (Some(AccessCursor::Rows(after)), _) => {
            let mut tx = pool.begin().await?;
            let page = rows_page(&mut tx, object, Vec::new(), after, size).await?;
            tx.commit().await?;
            return Ok(Some(page));
        }
    };

    // Read the owners. A full page of owners ends the page.
    let mut tx = pool.begin().await?;
    let mut entries = Vec::new();
    if let Some((workspace, after)) = owners {
        entries = store::owners(&mut tx, workspace, after, size.get() + 1).await?;
        if entries.len() > size.get() {
            entries.truncate(size.get());
            let next = entries
                .last()
                .map(|owner| AccessCursor::Owners(owner.actor_id));
            return Ok(Some(AccessPage { entries, next }));
        }
    }

    // Fill the page with the roles, and the profiles of their actors.
    let count = size.get() - entries.len();
    let Some(roles) = gate.roles(object, cursor.as_deref(), count).await? else {
        return Ok(None);
    };
    let ids: Vec<_> = roles.holders.iter().map(|holder| holder.actor).collect();
    let profiles = store::profiles(&mut tx, &ids).await?;
    for holder in roles.holders {
        let profile = profiles
            .iter()
            .find(|profile| profile.actor_id == holder.actor);
        if let Some(profile) = profile {
            entries.push(AccessEntry {
                role: holder.role,
                ..profile.clone()
            });
        }
    }
    if roles.more {
        tx.commit().await?;
        let next = Some(AccessCursor::Roles(roles.cursor.or(cursor)));
        return Ok(Some(AccessPage { entries, next }));
    }

    // Fill the page with the roles that are rows.
    let page = rows_page(&mut tx, object, entries, None, size).await?;
    tx.commit().await?;
    Ok(Some(page))
}

/// Fills a page with the roles that are rows on an object, after a grant.
async fn rows_page(
    tx: &mut scylla_extension::Tx<'_>,
    object: &Object,
    mut entries: Vec<AccessEntry>,
    after: Option<(ActorId, RoleId)>,
    size: PageSize,
) -> Result<AccessPage, Error> {
    let Ok(scope) = Uuid::parse_str(object.id()) else {
        return Ok(AccessPage {
            entries,
            next: None,
        });
    };
    let room = size.get().saturating_sub(entries.len());
    let mut rows = store::rows_on(tx, object.kind(), scope, after, room + 1).await?;
    let more = rows.len() > room;
    rows.truncate(room);
    let next = more
        .then(|| AccessCursor::Rows(rows.last().map(|grant| (grant.actor, grant.role)).or(after)));
    let ids: Vec<_> = rows.iter().map(|grant| grant.actor).collect();
    let profiles = store::profiles(tx, &ids).await?;
    for grant in rows {
        let profile = profiles
            .iter()
            .find(|profile| profile.actor_id == grant.actor);
        if let Some(profile) = profile {
            entries.push(AccessEntry {
                role: grant.role.to_string(),
                ..profile.clone()
            });
        }
    }
    Ok(AccessPage { entries, next })
}
