//! The sign-in with an identity provider: the start in the browser, the callback, and the
//! exchange of an ID token for a native client.

use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_extension::AccessError;
use sqlx::PgPool;

use super::github::{self, GitHubSettings};
use super::oidc::{OidcClients, OidcSettings};
use super::store::{self, FlowRow, ProviderRow};
use super::{join, ExternalIdentity};
use crate::features::identity::accounts::AccountContext;
use crate::features::identity::sessions::Opened;
use crate::infra::secret;
use crate::Error;

/// The kind of a provider, with its settings.
enum Kind {
    /// An OpenID Connect provider.
    Oidc(OidcSettings),
    /// GitHub or GitHub Enterprise Server.
    GitHub(GitHubSettings),
}

/// The end of a callback.
pub(crate) enum Outcome {
    /// A sign-in: the new session, and the page to go back to.
    SignedIn {
        /// The session.
        opened: Opened,
        /// The page of the web app.
        return_to: String,
    },
    /// A link to the signed-in user: the page to go back to.
    Linked {
        /// The page of the web app.
        return_to: String,
    },
}

/// The clients of the identity providers, with what the account routes share.
#[derive(Clone)]
pub struct Providers {
    /// What the account routes share.
    pub(crate) ctx: AccountContext,
    /// The HTTP client.
    http: reqwest::Client,
    /// The OIDC clients, with their discovery and keys.
    oidc: Arc<OidcClients>,
}

impl Providers {
    /// Makes the clients.
    pub fn new(ctx: AccountContext) -> Self {
        crate::infra::tls::install();
        let http = reqwest::Client::new();
        Self {
            oidc: Arc::new(OidcClients::new(http.clone())),
            http,
            ctx,
        }
    }

    /// Starts a sign-in, or a link for a signed-in user. Returns the URL of the provider and
    /// the token of the browser cookie.
    pub(crate) async fn start(
        &self,
        slug: &str,
        return_to: Option<&str>,
        link_user: Option<ActorId>,
    ) -> Result<(String, String), Error> {
        // Find the provider, and keep a return address of the web app only.
        let provider = self.provider(slug).await?;
        let settings = &self.ctx.settings;
        let return_to = return_to
            .filter(|url| settings.can_return_to(url))
            .unwrap_or(&settings.app_url)
            .to_owned();

        // Store the sign-in with new random values, for 10 minutes.
        let (state, verifier) = (secret::token(), secret::token());
        let (nonce, browser) = (secret::token(), secret::token());
        let flow = FlowRow {
            provider_id: provider.id,
            browser_hash: secret::hash(&browser),
            pkce_verifier: verifier.clone(),
            nonce: nonce.clone(),
            link_user,
            return_to,
        };
        let mut tx = self.ctx.pool.begin().await?;
        store::insert_flow(&mut tx, &secret::hash(&state), &flow).await?;
        tx.commit().await?;

        // Make the URL of the provider.
        let challenge = secret::pkce_challenge(&verifier);
        let redirect = self.redirect_uri(slug);
        let url = match kind(&provider)? {
            Kind::Oidc(oidc) => {
                let oidc_url = self
                    .oidc
                    .authorize_url(&oidc, &redirect, &state, &nonce, &challenge);
                oidc_url.await?
            }
            Kind::GitHub(github) => github::authorize_url(&github, &redirect, &state, &challenge)?,
        };
        Ok((url, browser))
    }

    /// Ends a sign-in or a link at the callback of the provider. The browser must be the one
    /// that started it, and a link needs the session of the same user.
    pub(crate) async fn callback(
        &self,
        slug: &str,
        code: &str,
        state: &str,
        browser: Option<&str>,
        signed_in: Option<ActorId>,
        user_agent: Option<&str>,
    ) -> Result<Outcome, Error> {
        // Take the sign-in of the state, for this browser and this provider.
        let mut tx = self.ctx.pool.begin().await?;
        let flow = store::take_flow(&mut tx, &secret::hash(state)).await?;
        tx.commit().await?;
        let flow = flow.ok_or_else(expired)?;
        let provider = self.provider(slug).await?;
        let same_browser =
            browser.is_some_and(|browser| secret::hash(browser) == flow.browser_hash);
        if flow.provider_id != provider.id || !same_browser {
            return Err(expired());
        }

        // Get the identity, then link it to the signed-in user, or sign in.
        let identity = self.identity(&provider, code, &flow).await?;
        if let Some(user) = flow.link_user {
            if signed_in != Some(user) {
                let reason = "sign in again to link this provider".to_owned();
                return Err(AccessError::Unauthenticated(reason).into());
            }
            join::link(&self.ctx, &provider, identity, user).await?;
            return Ok(Outcome::Linked {
                return_to: flow.return_to,
            });
        }
        let (_, opened) = join::sign_in(&self.ctx, &provider, identity, user_agent).await?;
        Ok(Outcome::SignedIn {
            opened,
            return_to: flow.return_to,
        })
    }

    /// Signs in with an ID token of an OIDC provider, for a native client. The issuer of the
    /// token names the provider.
    pub(crate) async fn sign_in_with_id_token(
        &self,
        id_token: &str,
        user_agent: Option<&str>,
    ) -> Result<(Actor, Opened), Error> {
        // Find the OIDC provider of the issuer of the token.
        let issuer = unverified_issuer(id_token).ok_or_else(invalid_id_token)?;
        let providers = list_enabled(&self.ctx.pool).await?;
        let found = providers
            .into_iter()
            .find_map(|provider| match kind(&provider) {
                Ok(Kind::Oidc(settings)) if settings.issuer == issuer => Some((provider, settings)),
                _ => None,
            });
        let Some((provider, settings)) = found else {
            let reason = "no identity provider has this issuer".to_owned();
            return Err(AccessError::Unauthenticated(reason).into());
        };

        // Check the token, then sign in.
        let identity = self.oidc.verify(&settings, id_token, None).await?;
        join::sign_in(&self.ctx, &provider, identity, user_agent).await
    }

    /// Exchanges the code of a callback for the identity of the person.
    async fn identity(
        &self,
        provider: &ProviderRow,
        code: &str,
        flow: &FlowRow,
    ) -> Result<ExternalIdentity, Error> {
        let redirect = self.redirect_uri(&provider.slug);
        let secret = self.client_secret(provider)?;
        let verifier = &flow.pkce_verifier;
        match kind(provider)? {
            Kind::Oidc(settings) => {
                let exchange =
                    self.oidc
                        .exchange(&settings, secret.as_deref(), code, verifier, &redirect);
                let id_token = exchange.await?;
                self.oidc
                    .verify(&settings, &id_token, Some(&flow.nonce))
                    .await
            }
            Kind::GitHub(settings) => {
                let secret = secret.ok_or_else(|| {
                    Error::Internal("a GitHub provider needs a client secret".to_owned())
                })?;
                github::identity(&self.http, &settings, &secret, code, verifier, &redirect).await
            }
        }
    }

    /// Returns a provider that is on, or `NOT_FOUND`.
    async fn provider(&self, slug: &str) -> Result<ProviderRow, Error> {
        let mut tx = self.ctx.pool.begin().await?;
        let provider = store::by_slug(&mut tx, slug).await?;
        tx.commit().await?;
        provider
            .filter(|provider| provider.enabled)
            .ok_or(Error::NotFound("provider"))
    }

    /// Returns the URL of the callback of a provider.
    fn redirect_uri(&self, slug: &str) -> String {
        format!("{}/auth/{slug}/callback", self.ctx.settings.api_url)
    }

    /// Decrypts the client secret of a provider, if it has one.
    fn client_secret(&self, provider: &ProviderRow) -> Result<Option<String>, Error> {
        let Some(sealed) = &provider.client_secret else {
            return Ok(None);
        };
        let sealer = self.ctx.sealer.as_ref().ok_or_else(|| Error::Config {
            name: "SCYLLA_SECRET_KEY",
            reason: "is required for a provider with a client secret".to_owned(),
        })?;
        sealer.open(sealed).map(Some).map_err(Error::Internal)
    }
}

/// Returns the providers that are on, in slug order.
pub(crate) async fn list_enabled(pool: &PgPool) -> Result<Vec<ProviderRow>, Error> {
    let mut tx = pool.begin().await?;
    let providers = store::list(&mut tx, true).await?;
    tx.commit().await?;
    Ok(providers)
}

/// Reads the kind and the settings of a provider.
fn kind(provider: &ProviderRow) -> Result<Kind, Error> {
    let settings = provider.settings.clone();
    let invalid = |error: serde_json::Error| {
        Error::Internal(format!(
            "the settings of {} are not valid: {error}",
            provider.slug
        ))
    };
    match provider.kind.as_str() {
        "oidc" => serde_json::from_value(settings)
            .map(Kind::Oidc)
            .map_err(invalid),
        "github" => serde_json::from_value(settings)
            .map(Kind::GitHub)
            .map_err(invalid),
        other => Err(Error::Internal(format!("unknown provider kind {other}"))),
    }
}

/// Reads the issuer of an ID token with no check. The check comes after, with the provider.
fn unverified_issuer(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims.get("iss")?.as_str().map(str::to_owned)
}

/// The error of an ID token that does not pass.
fn invalid_id_token() -> Error {
    AccessError::Unauthenticated("invalid ID token".to_owned()).into()
}

/// The error of a callback with an unknown or expired sign-in, or from another browser.
fn expired() -> Error {
    let reason = "the sign-in expired or comes from another browser: start again".to_owned();
    AccessError::Unauthenticated(reason).into()
}
