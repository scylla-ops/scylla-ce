//! The SQL of the identity providers, the external identities and the sign-in flows. Each
//! function runs in the transaction of the caller.

use scylla_domain::actors::ActorId;
use scylla_domain::users::ProviderId;
use scylla_extension::Tx;
use sqlx::types::Json;
use time::OffsetDateTime;

use crate::Error;

/// An identity provider, as the store returns it.
#[derive(Clone, Debug)]
pub struct ProviderRow {
    /// The id of the provider.
    pub id: ProviderId,
    /// The key of the provider in the URLs.
    pub slug: String,
    /// The kind: `oidc` or `github`.
    pub kind: String,
    /// The name to show.
    pub display_name: String,
    /// The settings of the kind, in JSON.
    pub settings: serde_json::Value,
    /// The client secret, encrypted with the key of the settings.
    pub client_secret: Option<Vec<u8>>,
    /// `true` if a new person can sign up with this provider.
    pub allow_sign_up: bool,
    /// `false` if the provider is off.
    pub enabled: bool,
}

/// An identity of a user, as the user sees it.
pub struct IdentityInfo {
    /// The slug of the provider.
    pub provider: String,
    /// The id of the person at the provider.
    pub subject: String,
    /// The email that the provider gave.
    pub email: Option<String>,
    /// The time of the link.
    pub linked_at: OffsetDateTime,
}

/// A sign-in that waits for the callback of its provider.
pub(super) struct FlowRow {
    /// The provider of the sign-in.
    pub(super) provider_id: ProviderId,
    /// The hash of the token of the browser cookie.
    pub(super) browser_hash: Vec<u8>,
    /// The PKCE verifier.
    pub(super) pkce_verifier: String,
    /// The nonce of the ID token.
    pub(super) nonce: String,
    /// The user that links the identity, for a link.
    pub(super) link_user: Option<ActorId>,
    /// Where the browser goes after the sign-in.
    pub(super) return_to: String,
}

/// Returns the provider of a slug, or `None`.
pub async fn by_slug(tx: &mut Tx<'_>, slug: &str) -> Result<Option<ProviderRow>, Error> {
    let row = sqlx::query!(
        r#"SELECT id, slug, kind, display_name, settings AS "settings: Json<serde_json::Value>",
                  client_secret, allow_sign_up, enabled
           FROM identity_providers WHERE slug = $1"#,
        slug
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| ProviderRow {
        id: ProviderId::from_db(row.id),
        slug: row.slug,
        kind: row.kind,
        display_name: row.display_name,
        settings: row.settings.0,
        client_secret: row.client_secret,
        allow_sign_up: row.allow_sign_up,
        enabled: row.enabled,
    }))
}

/// Returns the providers that are on, or all of them, in slug order.
pub async fn list(tx: &mut Tx<'_>, enabled_only: bool) -> Result<Vec<ProviderRow>, Error> {
    let rows = sqlx::query!(
        r#"SELECT id, slug, kind, display_name, settings AS "settings: Json<serde_json::Value>",
                  client_secret, allow_sign_up, enabled
           FROM identity_providers WHERE enabled OR NOT $1 ORDER BY slug"#,
        enabled_only
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ProviderRow {
            id: ProviderId::from_db(row.id),
            slug: row.slug,
            kind: row.kind,
            display_name: row.display_name,
            settings: row.settings.0,
            client_secret: row.client_secret,
            allow_sign_up: row.allow_sign_up,
            enabled: row.enabled,
        })
        .collect())
}

/// Adds a provider, or changes the provider of the same slug. Its identities stay.
pub async fn upsert(tx: &mut Tx<'_>, row: &ProviderRow) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO identity_providers
             (id, slug, kind, display_name, settings, client_secret, allow_sign_up, enabled)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (slug) DO UPDATE SET kind = $3, display_name = $4, settings = $5,
             client_secret = $6, allow_sign_up = $7, enabled = $8",
        row.id.to_db(),
        row.slug,
        row.kind,
        row.display_name,
        Json(&row.settings) as _,
        row.client_secret,
        row.allow_sign_up,
        row.enabled,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the user of an identity, or `None`.
pub(super) async fn user_of(
    tx: &mut Tx<'_>,
    provider: ProviderId,
    subject: &str,
) -> Result<Option<ActorId>, Error> {
    let user = sqlx::query_scalar!(
        "SELECT user_id FROM external_identities WHERE provider_id = $1 AND subject = $2",
        provider.to_db(),
        subject,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(user.map(ActorId::from_db))
}

/// Links an identity to a user.
pub(super) async fn link(
    tx: &mut Tx<'_>,
    provider: ProviderId,
    subject: &str,
    user: ActorId,
    email: Option<&str>,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO external_identities (provider_id, subject, user_id, email)
         VALUES ($1, $2, $3, $4)",
        provider.to_db(),
        subject,
        user.to_db(),
        email,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the identities of a user, in provider order.
pub(crate) async fn identities_of(
    tx: &mut Tx<'_>,
    user: ActorId,
) -> Result<Vec<IdentityInfo>, Error> {
    let rows = sqlx::query!(
        "SELECT p.slug, i.subject, i.email, i.linked_at
         FROM external_identities i JOIN identity_providers p ON p.id = i.provider_id
         WHERE i.user_id = $1 ORDER BY p.slug, i.subject",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| IdentityInfo {
            provider: row.slug,
            subject: row.subject,
            email: row.email,
            linked_at: row.linked_at,
        })
        .collect())
}

/// Removes an identity of a user. Returns `false` if the user has no such identity.
pub(crate) async fn unlink(
    tx: &mut Tx<'_>,
    user: ActorId,
    slug: &str,
    subject: &str,
) -> Result<bool, Error> {
    let result = sqlx::query!(
        "DELETE FROM external_identities i USING identity_providers p
         WHERE p.id = i.provider_id AND i.user_id = $1 AND p.slug = $2 AND i.subject = $3",
        user.to_db(),
        slug,
        subject,
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Stores a sign-in that waits for its callback, for 10 minutes.
pub(super) async fn insert_flow(
    tx: &mut Tx<'_>,
    state_hash: &[u8],
    flow: &FlowRow,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO sign_in_flows (state_hash, provider_id, browser_hash, pkce_verifier, nonce,
                                    link_user_id, return_to, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + interval '10 minutes')",
        state_hash,
        flow.provider_id.to_db(),
        flow.browser_hash,
        flow.pkce_verifier,
        flow.nonce,
        flow.link_user.map(ActorId::to_db),
        flow.return_to,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Takes the sign-in of a state: one use only. Returns `None` for an unknown or expired state.
pub(super) async fn take_flow(
    tx: &mut Tx<'_>,
    state_hash: &[u8],
) -> Result<Option<FlowRow>, Error> {
    sqlx::query!("DELETE FROM sign_in_flows WHERE expires_at < now()")
        .execute(&mut **tx)
        .await?;
    let row = sqlx::query!(
        "DELETE FROM sign_in_flows WHERE state_hash = $1
         RETURNING provider_id, browser_hash, pkce_verifier, nonce, link_user_id, return_to",
        state_hash
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| FlowRow {
        provider_id: ProviderId::from_db(row.provider_id),
        browser_hash: row.browser_hash,
        pkce_verifier: row.pkce_verifier,
        nonce: row.nonce,
        link_user: row.link_user_id.map(ActorId::from_db),
        return_to: row.return_to,
    }))
}
