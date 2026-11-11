//! The SQL of the GitHub App of the instance: the read of its row, with its secrets still sealed.

use scylla_domain::integrations::apps::VcsAppId;
use scylla_extension::Tx;

use crate::Error;

/// The row of the GitHub App of the instance. Each secret is sealed with the id of the row.
pub(crate) struct SealedApp {
    /// The id of the row.
    pub(crate) id: VcsAppId,
    /// The id of the App on GitHub.
    pub(crate) app_id: i64,
    /// The slug of the App, in its install URL.
    pub(crate) slug: String,
    /// The client id of the App.
    pub(crate) client_id: String,
    /// The sealed client secret.
    pub(crate) client_secret: Vec<u8>,
    /// The sealed private key, in PEM.
    pub(crate) private_key: Vec<u8>,
    /// The sealed secret of the webhooks.
    pub(crate) webhook_secret: Vec<u8>,
}

/// Returns the GitHub App of the instance, or `None` if the instance has none.
pub(crate) async fn github(tx: &mut Tx<'_>) -> Result<Option<SealedApp>, Error> {
    let row = sqlx::query!(
        "SELECT id, app_id, slug, client_id, sealed_client_secret, sealed_private_key,
                sealed_webhook_secret
         FROM vcs_apps WHERE provider = 'github'"
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| SealedApp {
        id: VcsAppId::from_db(row.id),
        app_id: row.app_id,
        slug: row.slug,
        client_id: row.client_id,
        client_secret: row.sealed_client_secret,
        private_key: row.sealed_private_key,
        webhook_secret: row.sealed_webhook_secret,
    }))
}
