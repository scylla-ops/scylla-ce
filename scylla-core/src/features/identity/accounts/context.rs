//! The settings of the accounts, and what the account routes share.

use std::sync::Arc;
use std::time::Duration;

use scylla_extension::{Gate, Grants, Hooks, Mailer};
use sqlx::PgPool;

use crate::infra::secret::Sealer;
use crate::{Config, SignUpMode};

/// The settings of the accounts and of the sessions.
#[derive(Clone, Debug)]
pub struct AccountSettings {
    /// The public URL of this server, with no trailing slash.
    pub api_url: String,
    /// The public URL of the web app, with no trailing slash.
    pub app_url: String,
    /// Who can create an account.
    pub sign_up_mode: SignUpMode,
    /// The lifetime of a session.
    pub session_ttl: Duration,
    /// The web origins of the app: a cookie refresh needs one, and a sign-in returns to one.
    pub web_origins: Vec<String>,
    /// `true` if the IP address of a sign-in comes from the last `X-Forwarded-For` address.
    pub trust_forwarded: bool,
}

impl AccountSettings {
    /// Reads the settings of the accounts.
    pub fn from_config(config: &Config) -> Self {
        Self {
            api_url: config.api_url.trim_end_matches('/').to_owned(),
            app_url: config.app_url.trim_end_matches('/').to_owned(),
            sign_up_mode: config.sign_up_mode,
            session_ttl: config.session_ttl,
            web_origins: config.cors_origins.clone(),
            trust_forwarded: config.trust_forwarded_headers,
        }
    }

    /// Returns `true` if the cookies need HTTPS.
    pub fn secure_cookies(&self) -> bool {
        self.api_url.starts_with("https://")
    }

    /// Makes a link of the web app with a one-time token in the fragment, so the token never
    /// goes to a server log or a `Referer` header: `<app>/<page>#token=<token>`.
    pub fn link(&self, page: &str, token: &str) -> String {
        format!("{}/{page}#token={token}", self.app_url)
    }

    /// Returns `true` if a browser can go back to this address after a sign-in: the web app,
    /// or one of its origins.
    pub fn can_return_to(&self, url: &str) -> bool {
        let allowed = std::iter::once(&self.app_url).chain(&self.web_origins);
        allowed
            .into_iter()
            .any(|origin| url == origin || url.starts_with(&format!("{origin}/")))
    }
}

/// What the account routes share.
#[derive(Clone)]
pub struct AccountContext {
    /// The database pool.
    pub pool: PgPool,
    /// The access gate, which signs the tokens and writes the roles.
    pub gate: Arc<dyn Gate>,
    /// The mailer.
    pub mailer: Arc<dyn Mailer>,
    /// The settings.
    pub settings: Arc<AccountSettings>,
    /// The key of the secrets, if the settings give one.
    pub sealer: Option<Arc<Sealer>>,
    /// The roles that an invitation can give.
    pub grants: Arc<Grants>,
    /// The hooks of the edition, for the invitations and the memberships.
    pub hooks: Arc<Hooks>,
}
