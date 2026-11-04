//! The logic of the workspaces: one transaction for each call.

use scylla_domain::authz::Object;
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::tenancy::grants::{MEMBER_KIND, OWNER};
use scylla_domain::tenancy::owners::{ListOwners, OwnerPage};
use scylla_domain::tenancy::slugs::Slug;
use scylla_domain::tenancy::workspaces::{
    ListWorkspaces, NewWorkspace, VisibleWorkspaces, Workspace, WorkspaceChange, WorkspaceId,
    WorkspacePage,
};
use scylla_domain::Page;
use scylla_extension::{CreateContext, Gate, Hooks, Rejection, WorkspaceFuture};
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Creates a workspace and makes its creator the first owner, in one transaction. The owners
/// are in Postgres, so the creation writes nothing to SpiceDB.
pub async fn create(pool: &PgPool, new: NewWorkspace) -> Result<Workspace, Error> {
    let mut tx = pool.begin().await?;
    let workspace = store::insert(&mut tx, &new).await?;
    tx.commit().await?;
    let (id, slug) = (workspace.id, &workspace.slug);
    tracing::info!(event = "workspace.created", workspace = %id, slug);
    Ok(workspace)
}

/// Returns a workspace, or `None` if no workspace has this id.
pub(super) async fn get(pool: &PgPool, id: WorkspaceId) -> Result<Option<Workspace>, Error> {
    let mut tx = pool.begin().await?;
    let workspace = store::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Returns the workspace of a slug, or `None` if no workspace has this slug.
pub(super) async fn get_by_slug(pool: &PgPool, slug: &Slug) -> Result<Option<Workspace>, Error> {
    let mut tx = pool.begin().await?;
    let workspace = store::find_by_slug(&mut tx, slug).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Finds a workspace, for `AccessService`: the workspace itself, or `None` if no workspace has
/// this id.
pub(super) fn find_workspace(pool: PgPool, id: WorkspaceId) -> WorkspaceFuture {
    Box::pin(async move {
        let mut tx = pool.begin().await?;
        let workspace = store::exists(&mut tx, id).await?;
        tx.commit().await?;
        Ok(workspace)
    })
}

/// Returns one page of the workspaces that a caller can read, oldest first.
pub(super) async fn list(
    pool: &PgPool,
    query: ListWorkspaces,
    visible: VisibleWorkspaces,
) -> Result<WorkspacePage, Error> {
    let mut tx = pool.begin().await?;
    let workspaces = store::list_visible(&mut tx, &visible, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(
        workspaces,
        query.page.size,
        |workspace| workspace.id,
    ))
}

/// Changes the name or the slug of a workspace, and returns it. A slug in use gives
/// `SLUG_IN_USE`.
pub(super) async fn update(
    pool: &PgPool,
    id: WorkspaceId,
    change: WorkspaceChange,
) -> Result<Workspace, Error> {
    let mut tx = pool.begin().await?;
    let workspace = store::update(&mut tx, id, &change)
        .await?
        .ok_or(Error::NotFound("workspace"))?;
    tx.commit().await?;
    Ok(workspace)
}

/// Deletes an empty workspace with its owners and its invitations, then removes the roles in
/// it. A project or an agent in it gives `WORKSPACE_NOT_EMPTY`. The lock of the row makes a
/// creation in the workspace at the same time wait, or the delete wait for it.
pub(super) async fn delete(pool: &PgPool, gate: &dyn Gate, id: WorkspaceId) -> Result<(), Error> {
    // Lock the workspace, then refuse if it holds a project or an agent.
    let mut tx = pool.begin().await?;
    if !store::lock(&mut tx, id).await? {
        return Err(Error::NotFound("workspace"));
    }
    if store::has_content(&mut tx, id).await? {
        let message = "the workspace holds projects or agents";
        return Err(Error::Rejected(Rejection::new(
            "WORKSPACE_NOT_EMPTY",
            message,
        )));
    }

    // Delete it with its owners and its invitations.
    store::delete_all(&mut tx, &[id]).await?;
    tx.commit().await?;
    tracing::info!(event = "workspace.deleted", workspace = %id);

    // Remove the roles after the commit. A role that stays reaches no workspace.
    if let Err(error) = gate.delete_object(&Object::of(id)).await {
        tracing::warn!(%error, workspace = %id, "the roles of a deleted workspace stay");
    }
    Ok(())
}

/// Makes a user an owner of a workspace. An owner stays an owner.
pub(super) async fn add_owner(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    workspace: WorkspaceId,
    user: ActorId,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    let hook = CreateContext::new(MEMBER_KIND, actor, workspace).with_member(user, None, OWNER);
    hooks.before_create(&mut tx, &hook).await?;
    store::add_owner(&mut tx, workspace, user).await?;
    tx.commit().await?;
    tracing::info!(event = "owner.added", %workspace, %user);
    Ok(())
}

/// Takes a user out of the owners of a workspace. The last owner stays: a workspace always has
/// one owner or more.
///
/// This holds when removals run at the same time. Each removal locks the row of the workspace
/// first, so the removals in one workspace run one after the other. In READ COMMITTED each
/// statement gets a new snapshot, so the count after the lock sees each removal that a previous
/// holder of the lock committed: two owners that take each other out at the same time leave
/// one owner. An addition does not take the lock. It can only make the count too low, so it can
/// cause a refusal, but never a workspace with no owner.
pub(super) async fn remove_owner(
    pool: &PgPool,
    workspace: WorkspaceId,
    user: ActorId,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;

    // Lock the workspace, so the removals in it run one after the other.
    if !store::lock(&mut tx, workspace).await? {
        return Err(Error::NotFound("workspace"));
    }

    // Remove the owner, then refuse if no owner stays: the drop of the transaction rolls back.
    store::remove_owner(&mut tx, workspace, user).await?;
    if store::count_owners(&mut tx, workspace).await? == 0 {
        let message = "a workspace keeps one owner or more";
        return Err(Error::Rejected(Rejection::new("LAST_OWNER", message)));
    }
    tx.commit().await?;
    tracing::info!(event = "owner.removed", %workspace, %user);
    Ok(())
}

/// Returns one page of the owners of a workspace, in user id order.
pub(super) async fn owners(pool: &PgPool, query: ListOwners) -> Result<OwnerPage, Error> {
    let mut tx = pool.begin().await?;
    let owners = store::owners(&mut tx, query.workspace_id, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(owners, query.page.size, |owner| {
        owner.user_id
    }))
}

/// Returns the workspaces that a caller can read and that refuse the personal tokens.
pub(crate) async fn refusing_personal_tokens(
    pool: &PgPool,
    visible: VisibleWorkspaces,
) -> Result<Vec<WorkspaceId>, Error> {
    let mut tx = pool.begin().await?;
    let ids = store::refusing_personal_tokens(&mut tx, &visible).await?;
    tx.commit().await?;
    Ok(ids)
}
