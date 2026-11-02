//! The logic of the actors.

use scylla_domain::actors::{Actor, ActorId, ActorKind};
use scylla_extension::Gate;
use sqlx::PgPool;

use super::store;
use crate::{access, Error};

/// Creates an actor of a kind, with its identity relationship.
pub async fn create(pool: &PgPool, gate: &dyn Gate, kind: ActorKind) -> Result<Actor, Error> {
    let mut tx = pool.begin().await?;
    let actor = store::insert(&mut tx, kind).await?;
    access::commit(tx, gate, &[Actor::IDENTITY.of(actor.id, actor.id)]).await?;
    Ok(actor)
}

/// Returns the actor with this id. An unknown actor gives `NotFound("actor")`.
pub async fn find(pool: &PgPool, id: ActorId) -> Result<Actor, Error> {
    let mut tx = pool.begin().await?;
    let actor = store::find(&mut tx, id).await?;
    tx.commit().await?;
    actor.ok_or(Error::NotFound("actor"))
}
