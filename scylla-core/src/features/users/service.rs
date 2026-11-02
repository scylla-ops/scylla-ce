//! The sign-in of a user: the ID token gives the identity, the first sign-in makes the user.

use scylla_domain::actors::Actor;
use scylla_domain::users::KIND;
use scylla_extension::Gate;
use sqlx::PgPool;

use super::{store, OidcVerifier};
use crate::features::actors;
use crate::{access, Error};

/// Returns the user of a valid ID token, and creates it at its first sign-in.
pub(crate) async fn sign_in(
    pool: &PgPool,
    gate: &dyn Gate,
    verifier: &OidcVerifier,
    id_token: &str,
) -> Result<Actor, Error> {
    let identity = verifier.verify(id_token).await?;
    let mut tx = pool.begin().await?;
    if let Some(id) = store::find(&mut tx, &identity.issuer, &identity.subject).await? {
        tx.commit().await?;
        return Ok(Actor { id, kind: KIND });
    }
    let actor = actors::insert(&mut tx, KIND).await?;
    store::insert(&mut tx, actor.id, &identity.issuer, &identity.subject).await?;
    let relations = [actors::identity(actor.id)];
    gate.write(&relations).await?;
    access::commit(tx, gate, &relations).await?;
    Ok(actor)
}
