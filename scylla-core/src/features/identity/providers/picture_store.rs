//! The SQL of the addresses of the photos of the identities. Each function runs in the
//! transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::ProviderId;
use scylla_extension::Tx;

use crate::Error;

/// Keeps the address of the photo that the provider gave at a sign-in or a link of an identity.
/// `None` is an identity with no photo.
pub(super) async fn set_picture(
    tx: &mut Tx<'_>,
    provider: ProviderId,
    subject: &str,
    picture: Option<&str>,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE external_identities SET picture_url = $3 WHERE provider_id = $1 AND subject = $2",
        provider.to_db(),
        subject,
        picture,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the address of the photo of an identity of a user. `None` is no such identity, and
/// `Some(None)` an identity with no photo.
pub(super) async fn picture_of(
    tx: &mut Tx<'_>,
    user: ActorId,
    provider: ProviderId,
    subject: &str,
) -> Result<Option<Option<String>>, Error> {
    let picture = sqlx::query_scalar!(
        "SELECT picture_url FROM external_identities
         WHERE user_id = $1 AND provider_id = $2 AND subject = $3",
        user.to_db(),
        provider.to_db(),
        subject,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(picture)
}
