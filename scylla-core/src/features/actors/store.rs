//! The SQL of the `actors` table.

use scylla_domain::actors::{Actor, ActorId, ActorKind};
use scylla_extension::Tx;

use crate::Error;

/// Inserts an actor of a kind, with the actor that owns it if there is one, and returns it.
pub(crate) async fn insert(
    tx: &mut Tx<'_>,
    kind: ActorKind,
    owner: Option<ActorId>,
) -> Result<Actor, Error> {
    let id = ActorId::generate();
    sqlx::query!(
        "INSERT INTO actors (id, kind, owner_id) VALUES ($1, $2, $3)",
        id.to_db(),
        kind.as_str(),
        owner.map(ActorId::to_db),
    )
    .execute(&mut **tx)
    .await?;
    Ok(Actor { id, kind })
}

/// Returns the actor with this id, or `None`.
pub(crate) async fn find(tx: &mut Tx<'_>, id: ActorId) -> Result<Option<Actor>, Error> {
    let kind = sqlx::query_scalar!("SELECT kind FROM actors WHERE id = $1", id.to_db())
        .fetch_optional(&mut **tx)
        .await?;
    Ok(kind.map(|kind| Actor {
        id,
        kind: ActorKind::from_name(kind),
    }))
}
