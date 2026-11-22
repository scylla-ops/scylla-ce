//! The logic of the library: one transaction for each call, with the hooks. `versions` is also
//! the door of an edition that reads the versions of a fragment in its own transaction.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::tenancy::library::{
    Fragment, FragmentId, FragmentPage, ListFragments, NewFragment, NewVersion, Version, KIND,
    VERSION_KIND,
};
use scylla_domain::Page;
use scylla_extension::{CreateContext, DeleteContext, Hooks, Rejection, Tx};
use sqlx::PgPool;

use super::store::versions::Nodes;
use super::store::{fragments, usages, versions as store_versions};
use crate::Error;

/// Returns the versions of a fragment with their nodes, the oldest first.
pub async fn versions(tx: &mut Tx<'_>, id: FragmentId) -> Result<Vec<Version>, Error> {
    let mut found = store_versions::of(tx, &[id.to_db()], Nodes::All).await?;
    Ok(found.remove(&id.to_db()).unwrap_or_default())
}

/// Creates a fragment with its first version if each `BeforeCreate` hook of `fragment` accepts
/// it, and returns it as the author reads it. A fragment of a package keeps the number of the
/// version of the package.
pub(super) async fn create(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    new: NewFragment,
) -> Result<Fragment, Error> {
    let mut tx = pool.begin().await?;
    let context = CreateContext::new(KIND, actor.clone(), new.workspace_id);
    hooks.before_create(&mut tx, &context).await?;
    let id = FragmentId::generate();
    fragments::insert(&mut tx, id, &new).await?;
    store_versions::insert(&mut tx, id, new.first_number(), actor.id, &new.version).await?;
    tx.commit().await?;
    tracing::info!(event = "fragment.created", fragment = %id, workspace = %new.workspace_id);
    get(pool, id, actor.id)
        .await?
        .ok_or(Error::NotFound("fragment"))
}

/// Adds the next version to a fragment if each `BeforeCreate` hook of `fragment_version` accepts
/// it. The hooks run while the fragment is locked, so the next number is the latest one plus 1.
pub(super) async fn publish_version(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    id: FragmentId,
    version: NewVersion,
) -> Result<Fragment, Error> {
    // Lock the fragment, refuse a linked fragment, then find the next number.
    let mut tx = pool.begin().await?;
    let fragment = fragments::find(&mut tx, id, true)
        .await?
        .ok_or(Error::NotFound("fragment"))?;
    if fragment.is_linked() {
        let message = "a linked fragment takes its versions from its package";
        return Err(Error::Rejected(Rejection::new("FRAGMENT_LINKED", message)));
    }
    let number = store_versions::latest(&mut tx, id).await? + 1;

    // Run the hooks, then insert the version.
    let context = CreateContext::new(VERSION_KIND, actor.clone(), fragment.workspace_id)
        .with_object(format!("fragment:{id}"));
    hooks.before_create(&mut tx, &context).await?;
    store_versions::insert(&mut tx, id, number, actor.id, &version).await?;
    tx.commit().await?;
    tracing::info!(event = "fragment.version_published", fragment = %id, number);
    get(pool, id, actor.id)
        .await?
        .ok_or(Error::NotFound("fragment"))
}

/// Deletes a fragment if each `BeforeDelete` hook accepts it, then runs the `AfterDelete` hooks.
/// The links of the pipelines stay.
pub(super) async fn delete(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    id: FragmentId,
) -> Result<(), Error> {
    // Lock the fragment, then run the hooks around the delete.
    let mut tx = pool.begin().await?;
    let fragment = fragments::find(&mut tx, id, true)
        .await?
        .ok_or(Error::NotFound("fragment"))?;
    let context = DeleteContext::new(KIND, actor, fragment.workspace_id)
        .with_object(format!("fragment:{id}"));
    hooks.before_delete(&mut tx, &context).await?;
    fragments::delete(&mut tx, id).await?;
    hooks.after_delete(&mut tx, &context).await?;
    tx.commit().await?;
    tracing::info!(event = "fragment.deleted", fragment = %id);
    Ok(())
}

/// Marks a fragment as a favorite of an actor, or removes the mark.
pub(super) async fn set_favorite(
    pool: &PgPool,
    id: FragmentId,
    actor: ActorId,
    favorite: bool,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    fragments::set_favorite(&mut tx, id, actor, favorite).await?;
    tx.commit().await?;
    Ok(())
}

/// Returns a fragment with no version, for a route check, or `None` if no fragment has this id.
pub(super) async fn find(pool: &PgPool, id: FragmentId) -> Result<Option<Fragment>, Error> {
    let mut tx = pool.begin().await?;
    let fragment = fragments::find(&mut tx, id, false).await?;
    tx.commit().await?;
    Ok(fragment)
}

/// Returns a fragment with the nodes of each version, as `reader` reads it.
pub(super) async fn get(
    pool: &PgPool,
    id: FragmentId,
    reader: ActorId,
) -> Result<Option<Fragment>, Error> {
    let mut tx = pool.begin().await?;
    let Some(fragment) = fragments::find(&mut tx, id, false).await? else {
        return Ok(None);
    };
    let mut full = complete(&mut tx, vec![fragment], reader, Nodes::All).await?;
    tx.commit().await?;
    Ok(full.pop())
}

/// Returns one page of the fragments of a workspace, with the nodes of their latest version.
pub(super) async fn list(pool: &PgPool, query: ListFragments) -> Result<FragmentPage, Error> {
    let mut tx = pool.begin().await?;
    let rows = fragments::list(&mut tx, query.workspace_id, query.page).await?;
    let page = Page::from_overfetch(rows, query.page.size, |fragment| fragment.id);
    let items = complete(&mut tx, page.items, query.reader, Nodes::Latest).await?;
    tx.commit().await?;
    Ok(Page {
        items,
        next_after: page.next_after,
    })
}

/// Adds the versions, the favorite of the reader and the usages to fragments of one workspace.
async fn complete(
    tx: &mut Tx<'_>,
    mut items: Vec<Fragment>,
    reader: ActorId,
    nodes: Nodes,
) -> Result<Vec<Fragment>, Error> {
    let Some(workspace) = items.first().map(|fragment| fragment.workspace_id) else {
        return Ok(items);
    };
    let ids: Vec<_> = items.iter().map(|fragment| fragment.id.to_db()).collect();
    let keys: Vec<_> = items.iter().map(|fragment| fragment.id).collect();
    let mut versions = store_versions::of(tx, &ids, nodes).await?;
    let favorites = fragments::favorites(tx, &ids, reader).await?;
    let mut usages = usages::of(tx, workspace, &keys).await?;
    for fragment in &mut items {
        fragment.versions = versions.remove(&fragment.id.to_db()).unwrap_or_default();
        fragment.favorite = favorites.contains(&fragment.id.to_db());
        fragment.usages = usages.remove(&fragment.id).unwrap_or_default();
    }
    Ok(items)
}
