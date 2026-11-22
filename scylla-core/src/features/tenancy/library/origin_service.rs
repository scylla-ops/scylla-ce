//! The logic of the fragments of a package: take a version of the package, skip it, hide it
//! from a fork, or make a linked fragment a fork. The client reads the registry, never the
//! server.

use scylla_domain::identity::actors::Actor;
use scylla_domain::tenancy::library::{Fragment, FragmentId, NewVersion, Origin, VERSION_KIND};
use scylla_extension::{CreateContext, Hooks, Rejection, Tx};
use sqlx::PgPool;

use super::service;
use super::store::{fragments, origins, versions};
use crate::Error;

/// Adds a version of its package to a linked fragment, with the number of the package. A number
/// that is not larger than each number of the fragment gives `FRAGMENT_VERSION_NOT_NEWER`.
pub(super) async fn accept(
    pool: &PgPool,
    hooks: &Hooks,
    actor: Actor,
    id: FragmentId,
    number: u32,
    version: NewVersion,
) -> Result<Fragment, Error> {
    // Lock the linked fragment, then refuse a number that is not newer.
    let mut tx = pool.begin().await?;
    let (fragment, _) = locked(&mut tx, id, true).await?;
    if number <= versions::latest(&mut tx, id).await? {
        let message = "the fragment has a version with this number or a larger one";
        return Err(refused("FRAGMENT_VERSION_NOT_NEWER", message));
    }

    // Run the hooks of a new version, then insert it with its number.
    let context = CreateContext::new(VERSION_KIND, actor.clone(), fragment.workspace_id)
        .with_object(format!("fragment:{id}"));
    hooks.before_create(&mut tx, &context).await?;
    versions::insert(&mut tx, id, number, actor.id, &version).await?;
    tx.commit().await?;
    tracing::info!(event = "fragment.origin_version_accepted", fragment = %id, number);
    read(pool, id, actor).await
}

/// Stops the proposal of a version of its package to a linked fragment.
pub(super) async fn skip(
    pool: &PgPool,
    actor: Actor,
    id: FragmentId,
    number: u32,
) -> Result<Fragment, Error> {
    change(pool, actor, id, true, |origin, _| {
        if !origin.skipped.contains(&number) {
            origin.skipped.push(number);
        }
    })
    .await
}

/// Hides a version of its package from a fork.
pub(super) async fn dismiss(
    pool: &PgPool,
    actor: Actor,
    id: FragmentId,
    number: u32,
) -> Result<Fragment, Error> {
    change(pool, actor, id, false, |origin, _| {
        origin.dismissed = Some(number)
    })
    .await
}

/// Makes a linked fragment a fork, from its latest version.
pub(super) async fn detach(pool: &PgPool, actor: Actor, id: FragmentId) -> Result<Fragment, Error> {
    change(pool, actor, id, true, |origin, latest| {
        origin.linked = false;
        origin.base_version = latest;
        origin.skipped.clear();
    })
    .await
}

/// Changes the origin of a locked fragment of the mode `linked`, and saves it.
async fn change(
    pool: &PgPool,
    actor: Actor,
    id: FragmentId,
    linked: bool,
    apply: impl FnOnce(&mut Origin, u32),
) -> Result<Fragment, Error> {
    let mut tx = pool.begin().await?;
    let (_, mut origin) = locked(&mut tx, id, linked).await?;
    let latest = versions::latest(&mut tx, id).await?;
    apply(&mut origin, latest);
    origins::save(&mut tx, id, &origin).await?;
    tx.commit().await?;
    read(pool, id, actor).await
}

/// Locks a fragment of a package in a mode. Another fragment gives `FRAGMENT_NOT_LINKED` or
/// `FRAGMENT_NOT_FORKED`.
async fn locked(
    tx: &mut Tx<'_>,
    id: FragmentId,
    linked: bool,
) -> Result<(Fragment, Origin), Error> {
    let fragment = fragments::find(tx, id, true)
        .await?
        .ok_or(Error::NotFound("fragment"))?;
    match fragment.origin.clone() {
        Some(origin) if origin.linked == linked => Ok((fragment, origin)),
        _ if linked => Err(refused(
            "FRAGMENT_NOT_LINKED",
            "the fragment is not linked to a package",
        )),
        _ => Err(refused(
            "FRAGMENT_NOT_FORKED",
            "the fragment is not a fork of a package",
        )),
    }
}

/// Returns the fragment as the actor reads it.
async fn read(pool: &PgPool, id: FragmentId, actor: Actor) -> Result<Fragment, Error> {
    service::get(pool, id, actor.id)
        .await?
        .ok_or(Error::NotFound("fragment"))
}

/// Makes a refusal with a code.
fn refused(code: &'static str, message: &str) -> Error {
    Error::Rejected(Rejection::new(code, message))
}
