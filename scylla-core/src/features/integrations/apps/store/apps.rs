//! The SQL of `vcs_apps`: the GitHub App of the instance, with its secrets still sealed, its read
//! with no secret, its insert and its delete.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::apps::{VcsApp, VcsAppId};
use scylla_domain::integrations::VcsProvider;
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

/// A new GitHub App of the instance, with its secrets sealed with the id of its row.
pub(crate) struct NewApp {
    /// The id of the row.
    pub(crate) id: VcsAppId,
    /// The id of the App on GitHub.
    pub(crate) app_id: i64,
    /// The slug of the App.
    pub(crate) slug: String,
    /// The display name.
    pub(crate) name: String,
    /// The login of the account that owns the App.
    pub(crate) owner: Option<String>,
    /// The page of the App on GitHub.
    pub(crate) web_url: String,
    /// The client id of the App.
    pub(crate) client_id: String,
    /// The sealed client secret.
    pub(crate) client_secret: Vec<u8>,
    /// The sealed private key, in PEM.
    pub(crate) private_key: Vec<u8>,
    /// The sealed secret of the webhooks.
    pub(crate) webhook_secret: Vec<u8>,
    /// The instance admin that made the App.
    pub(crate) created_by: ActorId,
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

/// Returns the GitHub App of the instance with no secret, or `None` if the instance has none.
pub(crate) async fn find(tx: &mut Tx<'_>) -> Result<Option<VcsApp>, Error> {
    let row = sqlx::query!(
        "SELECT id, app_id, slug, name, owner, web_url, client_id, created_by, created_at
         FROM vcs_apps WHERE provider = 'github'"
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| VcsApp {
        id: VcsAppId::from_db(row.id),
        provider: VcsProvider::GitHub,
        app_id: row.app_id,
        slug: row.slug,
        name: row.name,
        owner: row.owner,
        web_url: row.web_url,
        client_id: row.client_id,
        created_by: ActorId::from_db(row.created_by),
        created_at: row.created_at,
    }))
}

/// Keeps the GitHub App of the instance with its sealed secrets, made now. Returns `false`, with
/// no write, when the instance has an App.
pub(crate) async fn insert(tx: &mut Tx<'_>, app: &NewApp) -> Result<bool, Error> {
    let inserted = sqlx::query!(
        "INSERT INTO vcs_apps (id, provider, app_id, slug, name, owner, web_url, client_id,
                               sealed_client_secret, sealed_private_key, sealed_webhook_secret,
                               created_by)
         VALUES ($1, 'github', $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (provider) DO NOTHING",
        app.id.to_db(),
        app.app_id,
        app.slug,
        app.name,
        app.owner,
        app.web_url,
        app.client_id,
        app.client_secret,
        app.private_key,
        app.webhook_secret,
        app.created_by.to_db(),
    )
    .execute(&mut **tx)
    .await?
    .rows_affected();
    Ok(inserted > 0)
}

/// Locks the row of the GitHub App of the instance until the end of the transaction. Returns
/// its id, or `None` if the instance has no App.
pub(crate) async fn lock(tx: &mut Tx<'_>) -> Result<Option<VcsAppId>, Error> {
    let id = sqlx::query_scalar!("SELECT id FROM vcs_apps WHERE provider = 'github' FOR UPDATE")
        .fetch_optional(&mut **tx)
        .await?;
    Ok(id.map(VcsAppId::from_db))
}

/// Deletes an App. Its deliveries go with it.
pub(crate) async fn delete(tx: &mut Tx<'_>, id: VcsAppId) -> Result<(), Error> {
    sqlx::query!("DELETE FROM vcs_apps WHERE id = $1", id.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
