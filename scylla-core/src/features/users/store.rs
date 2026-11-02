//! The SQL of the `users` table.

use scylla_domain::actors::ActorId;
use scylla_extension::Tx;

/// Returns the user of an identity of an issuer, or `None`.
pub(super) async fn find(
    tx: &mut Tx<'_>,
    issuer: &str,
    subject: &str,
) -> Result<Option<ActorId>, sqlx::Error> {
    let id = sqlx::query_scalar!(
        "SELECT id FROM users WHERE issuer = $1 AND subject = $2",
        issuer,
        subject,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(id.map(ActorId::from_db))
}

/// Links an actor to an identity of an issuer.
pub(super) async fn insert(
    tx: &mut Tx<'_>,
    id: ActorId,
    issuer: &str,
    subject: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO users (id, issuer, subject) VALUES ($1, $2, $3)",
        id.to_db(),
        issuer,
        subject,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
