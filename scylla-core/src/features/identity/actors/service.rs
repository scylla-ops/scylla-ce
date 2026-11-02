//! The logic of the actors.

use scylla_domain::identity::actors::{Actor, ActorId, ActorKind};
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Creates an actor of a kind.
pub async fn create(pool: &PgPool, kind: ActorKind) -> Result<Actor, Error> {
    let mut tx = pool.begin().await?;
    let actor = store::insert(&mut tx, kind).await?;
    tx.commit().await?;
    Ok(actor)
}

/// Returns the actor with this id. An unknown actor gives `NotFound("actor")`.
pub async fn find(pool: &PgPool, id: ActorId) -> Result<Actor, Error> {
    let mut tx = pool.begin().await?;
    let actor = store::find(&mut tx, id).await?;
    tx.commit().await?;
    actor.ok_or(Error::NotFound("actor"))
}
