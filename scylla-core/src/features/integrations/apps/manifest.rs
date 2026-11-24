//! The manifest flow of GitHub for the App of the instance: a state of one use for one hour that
//! holds the id of the new row, the manifest and the page of GitHub, then the conversion of the
//! code and the row of the App with its secrets sealed.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::VcsProvider;
use scylla_extension::Rejection;
use serde_json::{json, Map, Value};
use time::{Duration, OffsetDateTime};
use url::Url;

use super::calls::{self, Converted};
use super::service::{self, Found, Shown};
use super::store::{self, states, NewApp};
use crate::features::integrations::connections::{checks, credentials};
use crate::features::integrations::IntegrationContext;
use crate::infra::secret::{self, Sealer};
use crate::infra::vcs::ProviderError;
use crate::Error;

/// The life of a state of the manifest flow. GitHub keeps the code of a new App for one hour.
const STATE_LIFE: Duration = Duration::hours(1);

/// The events of the App: the commits, the pull requests and the re-runs of the checks.
const EVENTS: [&str; 4] = ["push", "pull_request", "check_run", "check_suite"];

/// The longest name of a GitHub App, in characters.
const MAX_NAME: usize = 34;

/// What the browser posts to GitHub.
pub(super) struct Started {
    /// The page of GitHub, with the state.
    pub(super) post_url: String,
    /// The manifest of the App, as JSON.
    pub(super) manifest: String,
    /// The end of the validity of the state.
    pub(super) expires_at: OffsetDateTime,
}

/// Starts the creation of the App: keeps the hash of a new state that holds the id of the new
/// row, and returns the manifest and the page of GitHub for an organization, or for the account
/// of the person when `None`.
pub(super) async fn start(
    ctx: &IntegrationContext,
    organization: Option<&str>,
    caller: ActorId,
) -> Result<Started, Error> {
    // Refuse an App of the settings, a server with no key and a second App.
    service::refuse_settings(ctx)?;
    credentials::key(ctx.sealer.as_deref())?;
    let mut tx = ctx.pool.begin().await?;
    if store::find(&mut tx).await?.is_some() {
        return Err(service::app_exists());
    }

    // Keep the hash of a new state, with the id of the new row in it.
    let id = VcsAppId::generate();
    let state = format!("{id}.{}", secret::token());
    let expires_at = OffsetDateTime::now_utc() + STATE_LIFE;
    states::insert(&mut tx, &secret::hash(&state), caller, expires_at).await?;
    tx.commit().await?;

    // Make the manifest and the page of GitHub.
    let base = ctx.settings.github_web_url.as_str();
    let page = match organization {
        Some(organization) => format!("{base}/organizations/{organization}/settings/apps/new"),
        None => format!("{base}/settings/apps/new"),
    };
    let mut url = Url::parse(&page).map_err(|_| Error::Internal(format!("{page} is not a URL")))?;
    url.query_pairs_mut().append_pair("state", &state);
    Ok(Started {
        post_url: url.into(),
        manifest: document(ctx, id).to_string(),
        expires_at,
    })
}

/// Completes the creation: uses the state, gets the new App from GitHub with the code, and keeps
/// it with its secrets sealed with the id of its row.
pub(super) async fn complete(
    ctx: &IntegrationContext,
    code: &str,
    state: &str,
    caller: ActorId,
) -> Result<Shown, Error> {
    // Refuse an App of the settings and a server with no key.
    service::refuse_settings(ctx)?;
    let sealer = credentials::key(ctx.sealer.as_deref())?;

    // Use the state one time, and read the id of the new row in it.
    let id = state
        .split_once('.')
        .and_then(|(id, _)| VcsAppId::parse(id).ok())
        .ok_or_else(state_expired)?;
    let mut tx = ctx.pool.begin().await?;
    let taken = states::take(&mut tx, &secret::hash(state), caller).await?;
    let exists = store::find(&mut tx).await?.is_some();
    tx.commit().await?;
    if !taken {
        return Err(state_expired());
    }
    if exists {
        return Err(service::app_exists());
    }

    // Get the App from GitHub.
    let converted = calls::convert(&ctx.clients, code)
        .await
        .map_err(|error| match error {
            ProviderError::NotFound => state_expired(),
            error => credentials::provider_failed(VcsProvider::GitHub, error),
        })?;

    // Seal its secrets with the id of the row, and keep it.
    let slug = converted.slug.clone();
    let new = sealed(sealer, id, caller, converted)?;
    let mut tx = ctx.pool.begin().await?;
    let inserted = store::insert(&mut tx, &new).await?;
    let app = store::find(&mut tx).await?;
    tx.commit().await?;
    let Some(app) = app.filter(|_| inserted) else {
        tracing::warn!(%slug, "a second GitHub App stays on GitHub: delete it there");
        return Err(service::app_exists());
    };
    tracing::info!(
        event = "vcs.app_created",
        app = %id,
        slug = %app.slug,
        owner = app.owner.as_deref().unwrap_or_default(),
    );
    Ok(Shown {
        found: Found::Instance(app),
        last: None,
    })
}

/// Returns the manifest of the App: its name and its URLs on this server, its permissions and
/// its events. The webhook URL has the id of the new row.
fn document(ctx: &IntegrationContext, id: VcsAppId) -> Value {
    let app_url = ctx.urls.app_url.trim_end_matches('/');
    let api_url = ctx.urls.api_url.trim_end_matches('/');
    let callback = format!("{app_url}/integrations/github/callback");
    let permissions: Map<String, Value> = checks::WANTED
        .iter()
        .map(|(name, access)| ((*name).to_owned(), Value::from(*access)))
        .collect();
    json!({
        "name": name(app_url),
        "url": app_url,
        "description": "Runs the Scylla pipelines on the commits, and reports their results.",
        "public": false,
        "hook_attributes": {
            "url": format!("{api_url}/integrations/github/{id}/webhook"),
            "active": true,
        },
        "redirect_url": format!("{app_url}/integrations/github/manifest"),
        "callback_urls": [callback],
        "setup_url": callback,
        "setup_on_update": true,
        "request_oauth_on_install": true,
        "default_permissions": permissions,
        "default_events": EVENTS,
    })
}

/// Returns the name of the App: `Scylla` and the host of the server, at most 34 characters.
fn name(app_url: &str) -> String {
    let host = Url::parse(app_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_default();
    let name = format!("Scylla {host}");
    name.trim().chars().take(MAX_NAME).collect()
}

/// Seals the secrets of a new App with the id of its row. GitHub gives a webhook secret to an
/// App with a webhook.
fn sealed(
    sealer: &Sealer,
    id: VcsAppId,
    caller: ActorId,
    converted: Converted,
) -> Result<NewApp, Error> {
    let webhook_secret = converted
        .webhook_secret
        .ok_or_else(|| Error::Internal("GitHub gave no webhook secret".to_owned()))?;
    let data = id.to_bytes();
    let seal = |text: &str| sealer.seal_with(text.as_bytes(), &data);
    Ok(NewApp {
        id,
        app_id: converted.id,
        slug: converted.slug,
        name: converted.name,
        owner: converted.owner.map(|owner| owner.login),
        web_url: converted.html_url,
        client_id: converted.client_id,
        client_secret: seal(converted.client_secret.expose()),
        private_key: seal(converted.pem.expose()),
        webhook_secret: seal(webhook_secret.expose()),
        created_by: caller,
    })
}

/// The refusal of a state that is unknown, used, old or of another person, or of a code that
/// GitHub does not know.
fn state_expired() -> Error {
    let message = "the link expired: create the App again";
    Error::Rejected(Rejection::new("STATE_EXPIRED", message))
}
