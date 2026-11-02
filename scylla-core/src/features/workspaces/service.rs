//! The logic of the workspaces: one transaction for each call.

use scylla_domain::actors::ActorId;
use scylla_domain::page::Page;
use scylla_domain::workspaces::{
    ListOwners, ListWorkspaces, NewWorkspace, OwnerPage, VisibleWorkspaces, Workspace, WorkspaceId,
    WorkspacePage,
};
use scylla_extension::Rejection;
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Creates a workspace and makes its creator the first owner, in one transaction. The owners
/// are in Postgres, so the creation writes nothing to SpiceDB.
pub async fn create(pool: &PgPool, new: NewWorkspace) -> Result<Workspace, Error> {
    let mut tx = pool.begin().await?;
    let workspace = store::insert(&mut tx, &new).await?;
    tx.commit().await?;
    Ok(workspace)
}

/// Returns a workspace, or `None` if no workspace has this id.
pub(super) async fn get(pool: &PgPool, id: WorkspaceId) -> Result<Option<Workspace>, Error> {
    let mut tx = pool.begin().await?;
    let workspace = store::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(workspace)
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

/// Makes a user an owner of a workspace. An owner stays an owner.
pub(super) async fn add_owner(
    pool: &PgPool,
    workspace: WorkspaceId,
    user: ActorId,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    store::add_owner(&mut tx, workspace, user).await?;
    tx.commit().await?;
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
