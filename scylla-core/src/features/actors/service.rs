//! The logic of the actors.

use scylla_domain::actors::{Actor, ActorId, ActorKind};
use scylla_extension::{Gate, Object, Relation};
use sqlx::PgPool;

use super::store;
use crate::{access, Error};

/// Returns the relation that lets an actor act for itself.
pub fn identity(id: ActorId) -> Relation {
    Relation {
        object: Object::new("actor", id),
        relation: "identity",
        subject: Object::new("actor", id),
    }
}

/// Creates an actor of a kind, with its identity relation.
pub async fn create(pool: &PgPool, gate: &dyn Gate, kind: ActorKind) -> Result<Actor, Error> {
    let mut tx = pool.begin().await?;
    let actor = store::insert(&mut tx, kind).await?;
    let relations = [identity(actor.id)];
    gate.write(&relations).await?;
    access::commit(tx, gate, &relations).await?;
    Ok(actor)
}

/// Returns the actor with this id. An unknown actor gives `NotFound("actor")`.
pub async fn find(pool: &PgPool, id: ActorId) -> Result<Actor, Error> {
    let mut tx = pool.begin().await?;
    let actor = store::find(&mut tx, id).await?;
    tx.commit().await?;
    actor.ok_or(Error::NotFound("actor"))
}
