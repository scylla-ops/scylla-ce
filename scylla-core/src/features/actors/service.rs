//! The logic of the actors.

use scylla_domain::actors::{Actor, ActorKind};
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
