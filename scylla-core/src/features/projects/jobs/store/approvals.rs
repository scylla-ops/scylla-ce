//! The SQL of the approval of a job from a fork: the name of the actor that rejects it.

use scylla_domain::identity::actors::ActorId;
use scylla_extension::Tx;

use crate::Error;

/// Returns the name of an actor: the display name of a user or the name of an App, or `None`
/// for another actor.
pub(crate) async fn name_of(tx: &mut Tx<'_>, actor: ActorId) -> Result<Option<String>, Error> {
    let name = sqlx::query_scalar!(
        "SELECT COALESCE((SELECT display_name FROM users WHERE id = $1),
                         (SELECT name FROM apps WHERE id = $1))",
        actor.to_db(),
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(name)
}
