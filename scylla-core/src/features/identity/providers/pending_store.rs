//! The SQL of the links that wait for the password of an account. Each function runs in the
//! transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::ProviderId;
use scylla_extension::Tx;
use sqlx::types::Json;

use super::store::ProviderRow;
use crate::Error;

/// A link that waits for the password of an account, as it is stored.
pub(super) struct NewPendingLink<'a> {
    /// The provider.
    pub(super) provider: ProviderId,
    /// The id of the person at the provider.
    pub(super) subject: &'a str,
    /// The account that has the email.
    pub(super) user: ActorId,
    /// The verified email that the provider gave.
    pub(super) email: &'a str,
    /// The name that the provider gave.
    pub(super) display_name: Option<&'a str>,
    /// Where the browser goes at the end.
    pub(super) return_to: &'a str,
}

/// A link that waits for the password of an account.
pub(super) struct PendingLink {
    /// The provider, which is on.
    pub(super) provider: ProviderRow,
    /// The id of the person at the provider.
    pub(super) subject: String,
    /// The account that has the email.
    pub(super) user: ActorId,
    /// The verified email that the provider gave.
    pub(super) email: String,
    /// The name that the provider gave.
    pub(super) display_name: Option<String>,
    /// Where the browser goes at the end.
    pub(super) return_to: String,
}

/// Stores a link that waits, for 10 minutes. It first deletes the links that expired.
pub(super) async fn insert_pending_link(
    tx: &mut Tx<'_>,
    token_hash: &[u8],
    link: &NewPendingLink<'_>,
) -> Result<(), Error> {
    sqlx::query!("DELETE FROM pending_links WHERE expires_at < now()")
        .execute(&mut **tx)
        .await?;
    sqlx::query!(
        "INSERT INTO pending_links (token_hash, provider_id, subject, user_id, email,
                                    display_name, return_to, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + interval '10 minutes')",
        token_hash,
        link.provider.to_db(),
        link.subject,
        link.user.to_db(),
        link.email,
        link.display_name,
        link.return_to,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns a link that waits, if it has not expired and its provider is on.
pub(super) async fn find_pending_link(
    tx: &mut Tx<'_>,
    token_hash: &[u8],
) -> Result<Option<PendingLink>, Error> {
    let row = sqlx::query!(
        r#"SELECT p.id, p.slug, p.kind, p.display_name AS provider_name,
                  p.settings AS "settings: Json<serde_json::Value>",
                  p.client_secret, p.allow_sign_up, p.enabled,
                  l.subject, l.user_id, l.email, l.display_name, l.return_to
           FROM pending_links l JOIN identity_providers p ON p.id = l.provider_id
           WHERE l.token_hash = $1 AND l.expires_at > now() AND p.enabled"#,
        token_hash
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| PendingLink {
        provider: ProviderRow {
            id: ProviderId::from_db(row.id),
            slug: row.slug,
            kind: row.kind,
            display_name: row.provider_name,
            settings: row.settings.0,
            client_secret: row.client_secret,
            allow_sign_up: row.allow_sign_up,
            enabled: row.enabled,
        },
        subject: row.subject,
        user: ActorId::from_db(row.user_id),
        email: row.email,
        display_name: row.display_name,
        return_to: row.return_to,
    }))
}

/// Deletes a link that waits, if it has not expired. Returns `false` if it is not there.
pub(super) async fn delete_pending_link(tx: &mut Tx<'_>, token_hash: &[u8]) -> Result<bool, Error> {
    let result = sqlx::query!(
        "DELETE FROM pending_links WHERE token_hash = $1 AND expires_at > now()",
        token_hash
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}
